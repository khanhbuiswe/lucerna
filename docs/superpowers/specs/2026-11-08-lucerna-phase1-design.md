# Lucerna — Phase 1: Core Viewer

- **Date:** 2026-11-08
- **Status:** Draft, awaiting approval
- **Product name:** Lucerna (Latin: "lamp")
- **CLI command:** `luc`
- **Root crate:** `lucerna`

## 1. Background and goals

### 1.1 What the product is

Lucerna is a leaner and faster take on VS Code, written in Rust and released as open source under a permissive license (MIT OR Apache-2.0).

The target users write code mostly through agents running in a terminal, such as Claude Code or Codex. They open the editor mainly to **view**: browse the project, read code, and follow what the agent just changed. Those who want to code directly in Lucerna can still edit fully, but editing belongs to a later phase.

### 1.2 What was decided

- **No AI features** in the editor: no chat, no ghost text, no MCP.
- **VS Code-like experience:** layout, Quick Open, `settings.json`, `keybindings.json`, default shortcuts.
- **Extensions use Zed's format** (WASM, tree-sitter, theme JSON), not the VS Code format. This belongs to Phase 2.
- **UI written in GPUI** (Apache-2.0). Everything else is written from scratch, with no dependency on any of Zed's GPL crates.

### 1.3 Overall roadmap

This spec describes Phase 1 **only**. Each later phase has its own spec.

| # | Phase | Result |
|---|---|---|
| **1** | **Core viewer** (this spec) | Can replace VS Code for viewing code |
| 2 | Zed extension host | Themes, icon themes, languages and grammars, snippets from Zed extensions |
| 3 | Understanding code via LSP | Definitions, references, hover, diagnostics, outline, breadcrumbs, Quick Open `@` and `#` |
| 3b | WASM host for Zed extensions | Language servers from `extension.wasm`. PHP, Ruby, Elixir, Vue… already run through the language server table (`2026-11-11-lsp-server-table-design.md`) once the user has installed the server |
| 4 | Full editing ([spec](2026-11-11-lucerna-phase4-editing-design.md), 4.1, 4.2, 4.3 and 4.4 done) | Multi-cursor, undo, conflict handling with agents, completion, rename, format |
| 5 | Extensions | Git gutter, diff view, integrated terminal, settings UI (Git view: 5.1 core Git — Explorer colors, gutter; 5.2 diff tab — Uncommitted Changes, Changes since ‹branch›, single-file diff; both done; branch switching from the title bar done). Settings UI not done yet |
| V1 | Vim navigation (done) | Enabled with `vim.enabled` in settings. Read-only operations only: motions, search, scrolling, counts, marks, selecting with visual mode then copying |
| V2 | Vim editing (done right after steps 4.2 and 4.4, before 4.3, 4.5, 4.6). **V2a and V2b done** | Insert mode, the `d`/`c`/`y` operators, text objects, `.`, macros, registers, part of the `:` commands |
| P3 | Fix the bugs still open after the 0.1.0 release ([spec](2026-11-17-p3-fixes-design.md)), done | T1-1 (wrong bytes for the encoding) and every P3 bug from the pre-release review |
| MD | Markdown preview (done, still waiting on two checks: compare the look with Zed, measure RAM after closing a tab with an image) ([spec](2026-11-13-markdown-preview-design.md)) | View rendered `.md` in its own tab, scroll synced with the source tab, local images and links, open HTML files in the browser |
| IMG | Image preview (done and checked in the app; for RAM after closing a large image see [research §6](../../research/2026-11-18-image-preview.md)) ([spec](2026-11-18-image-preview-design.md)) | Open png, jpg, gif, webp, bmp, ico, tiff and svg images in their own tab, auto-reload when the file changes, SVG opens as an image by default |

Vim mode is a built-in feature of the editor core, not an extension, the same way Zed does it. See the research in `docs/research/2026-11-09-vim-mode.md`.

**Principle for every phase:** before planning any feature, always look at how VS Code and Zed implemented it. See `docs/design-principles.md` for details.

### 1.4 Success criteria for Phase 1

Phase 1 is done when the author uses Lucerna instead of VS Code to read code every day for a week without having to reopen VS Code to view code, and it meets the performance targets in section 6.

## 2. Scope

### 2.1 In Phase 1

- Run `luc [path]` to open a folder or a file. Each run opens one window.
- **Layout:**
  - An activity bar with two items, Explorer and Search.
  - The sidebar can be hidden or shown with Cmd+B.
  - The editor area can be split into several panes (Cmd+\), each with tabs.
  - Status bar.
- **Explorer:**
  - A folder tree that respects `.gitignore` and `files.exclude`.
  - Updates itself when files on disk change.
  - Marks files just changed from outside, which can be turned off via `explorer.decorations.externalChanges`.
  - Automatically highlights the open file, per `explorer.autoReveal`.
- **Tabs:**
  - Preview tab: italic text, replaced by the next file opened. Double-click to keep the tab.
  - Cmd+W closes a tab.
  - Ctrl+Tab switches back and forth between recent tabs.
- **Code view:**
  - Editable from Phase 4: see [2026-11-11-lucerna-phase4-editing-design.md](2026-11-11-lucerna-phase4-editing-design.md).
  - Line numbers, syntax coloring, smooth scrolling, togglable word wrap.
  - Select text and copy (Cmd+C).
  - Find in file (Cmd+F), with options for case sensitivity, whole word and regex.
  - Minimap, which can be turned off via `editor.minimap.enabled`.
  - Indent-based folding.
  - Highlighting of matching bracket pairs.
- **Quick Open:**
  - A single widget.
  - No prefix: find a file (Cmd+P).
  - `>`: run a command (Cmd+Shift+P).
  - `:`: jump to a line (Ctrl+G).
- **Search** (Cmd+Shift+F):
  - Search the whole project, with options for case sensitivity, whole word, regex, and include/exclude globs.
  - Results are shown in the sidebar, grouped by file.
- **Settings and keybindings:**
  - `settings.json` and `keybindings.json` with VS Code semantics.
  - Commands to open these two files are in the command palette.
- **Theme:**
  - Reads theme JSON following the Zed v0.2.0 schema.
  - Ships with One Dark and One Light (MIT, taken from Zed's `assets/themes` folder, with the license file).
- **Languages:**
  - Ships with built-in tree-sitter grammars for Rust, TypeScript, TSX, JavaScript, Python, Go, JSON, Markdown (including inline), TOML, YAML, Bash, HTML and CSS.
  - Queries are taken from each grammar's original repo.
- **Saving workspace state:** layout, pane sizes, open tabs, scroll positions.

### 2.2 Not in Phase 1

Text editing, LSP, extensions, git, terminal, debugging, a settings editing UI, multi-platform support, remote, opening several root folders in one window, auto-update, telemetry (Lucerna never has telemetry).

### 2.3 Platform

macOS 13 and later only, running on Apple Silicon and Intel. Linux and Windows come after Phase 1.

## 3. Architecture

### 3.1 Principles

1. **The UI thread never waits** for I/O, parsing or search. Heavy work runs on a background thread over an immutable snapshot, then sends the result back to the UI.
2. **Only the `workbench` crate depends on GPUI.** The other crates are pure Rust, testable without a window.
3. **No GPL code.** Only data formats (theme JSON, `.scm` files, keymap structure) are read, as an interface; no GPL source code of Zed is copied. CI enforces this with `cargo-deny`.
4. **Written so editing can be added later.** `text` uses a rope from the start, even though Phase 1 is read-only. That way Phase 4 does not have to change the data structures.

### 3.2 Crates

All live in one Cargo workspace, in the `crates/` folder.

| Crate | Responsibility | Main dependencies |
|---|---|---|
| `lucerna-text` | Rope buffer, cheap snapshots (clone via `Arc`), line/column ↔ byte conversion, encoding and line-ending detection | `ropey`, `encoding_rs` |
| `lucerna-syntax` | Language registry (by extension and file name), background tree-sitter parsing, highlight computation for a range of lines, folding | `tree-sitter`, grammar crates |
| `lucerna-project` | Background directory scan, file tree, watcher, fuzzy file search, content search | `ignore`, `notify`, `grep-searcher`, `grep-regex`, `nucleo-matcher` |
| `lucerna-settings` | Merging settings layers, default schema, reading keybindings, evaluating `when` clauses | `serde`, `serde_json`, `jsonc-parser` |
| `lucerna-theme` | Reading Zed theme JSON, mapping tree-sitter capture names to styles | `serde`, `palette` |
| `lucerna-workbench` | All of the UI: layout, explorer, tabs, code view, Quick Open, search panel, status bar, state saving | `gpui` (git, pinned by revision) |
| `lucerna` (bin `luc`) | Reading CLI arguments, initializing logging, wiring the crates together | `clap`, `tracing` |

**Pinning the GPUI version.** `gpui` is taken via git from the `zed-industries/zed` repo, pinned to a specific commit. That commit is chosen when implementation starts and recorded in `Cargo.toml`. Update it only in a separate PR, with re-testing.

### 3.3 Interfaces between crates

- `project::Worktree` provides:
  - `snapshot()`: an immutable list of files.
  - A stream of `WorktreeEvent` events, including `Added`, `Removed`, `Modified` and `Rescanned`.
- `text::Buffer` provides:
  - `load(path)`: returns `Result<Buffer, LoadError>`.
  - `snapshot()`: returns `BufferSnapshot`.
  - `reload(new_contents)`: returns a `Reload` saying which lines changed, used to keep the scroll position.
- `syntax::SyntaxMap` provides:
  - `parse(snapshot)`: an asynchronous function.
  - `highlights(snapshot, line_range)`: returns a list of `(byte_range, capture_name)`.
- `theme::Theme::style_for(capture_name)`: returns `HighlightStyle`, falling back by capture name (`function.method` → `function`).
- `settings::Settings::get::<T>(key, language)`: returns the value merged across layers. Whenever settings change, it emits a change event.
- `workbench` uses only the interfaces above. It does not know how the rope or tree-sitter work inside.

## 4. Data flow

### 4.1 Startup

1. `luc` reads the arguments, initializes `tracing`, then loads settings (default → user → workspace) and the theme.
2. GPUI opens the window and restores the workspace state from a JSON file. If the file is missing or corrupt, the default layout is used.
3. `project` starts scanning the folder in the background. The file tree appears gradually in batches of about 16 ms during the scan.
4. The active tab's file is read and shown right away, not yet colored. Colors appear when parsing finishes.
5. Other tabs only have their files read when the user switches to them.

### 4.2 Opening and viewing a file

1. Read the file and detect the encoding: UTF-8 first, then check for a BOM, and finally guess with `encoding_rs`. A binary file is recognized by a NUL byte in the first 8 KB.
2. Create the `Buffer` and `BufferSnapshot`.
3. Tree-sitter parse: wait synchronously for at most 1 ms; if that is exceeded, leave it to the background thread and redraw when done.
4. Only compute highlights for the lines being displayed, plus a 100-line margin above and below.
5. **Large-file thresholds** (configurable in settings):
   - A file over 20 MB or over 300,000 lines: no coloring, no minimap, no folding.
   - A line longer than 10,000 characters: render only the beginning, followed by a `…` mark.
   - A file over 512 MB: not opened, only a notice is shown.

### 4.3 When a file changes from outside

1. `notify` (FSEvents on macOS) sends an event. `project` merges events within 50 ms.
2. If `notify` reports a queue overflow, or there are more than 1,000 events in one batch (for example when an agent runs `git checkout`), `project` rescans everything in the background and emits `Rescanned`.
3. The file tree is updated and the changed file is marked. The mark disappears by itself after 10 seconds or when the user opens the file.
4. For a file that is open:
   - Re-read the file.
   - Call `Buffer::reload`.
   - Keep the top line of the viewport, shifted by the number of lines added or removed above it.
   - Re-parse.
5. If an open file is deleted: the tab switches to a "deleted" state (name struck through), and the content remains for viewing. If the file reappears, the tab returns to normal.
6. Goal: under 100 ms from the moment the file is written to the screen updating.

### 4.4 Quick Open

- **Cmd+P:**
  - Fuzzy match with `nucleo-matcher` over a snapshot of the file list.
  - Runs in the background, returns the first 50 results. On each keystroke, the previous match run is cancelled.
  - Priority order: recently opened files, then match score, then shorter path.
- **`>`:** lists the commands in the command registry, shown with their shortcuts.
- **`:N` or `:N:C`:** jump to line N, column C, in the open file.
- Enter opens the file as a preview tab. Cmd+Enter opens the file in the adjacent pane.

### 4.5 Project-wide search

- Runs on multiple threads using `ignore::WalkParallel` combined with `grep-searcher`. Respects `.gitignore`, `search.exclude` and hand-typed globs.
- Results stream into the panel gradually, grouped by file.
- Limit of 20,000 results, with a notice when truncated.
- Changing the query cancels the old search (150 ms debounce).
- Clicking a result: opens the file and highlights the match.

## 5. Settings, keybindings and commands

### 5.1 Paths

| Kind | Path |
|---|---|
| User settings | `~/.config/lucerna/settings.json` |
| User keybindings | `~/.config/lucerna/keybindings.json` |
| User themes | `~/.config/lucerna/themes/*.json` |
| Workspace settings | `<root>/.lucerna/settings.json` |
| Workspace state | `~/Library/Application Support/Lucerna/workspaces/<sha256(root)>.json` |
| Log | `~/Library/Logs/Lucerna/` |

### 5.2 Settings

- JSON with comments (JSONC) format.
- Merged in order: default → user → workspace. Per-language overrides use the key `"[rust]": { … }`.
- Key names stay exactly as in VS Code when an equivalent concept exists. The Phase 1 key list:
  - `editor.fontFamily`, `editor.fontSize`, `editor.lineHeight`, `editor.wordWrap`, `editor.minimap.enabled`, `editor.renderWhitespace`, `editor.tabSize`
  - `files.exclude`, `search.exclude`
  - `workbench.colorTheme`, `workbench.editor.enablePreview`
  - `explorer.autoReveal`, `explorer.decorations.externalChanges`
  - `lucerna.largeFile.maxBytes`, `lucerna.largeFile.maxLines`
- Settings are reloaded automatically when the file changes on disk.

### 5.3 Keybindings

- Same format as VS Code: an array of elements `{ "key": "cmd+p", "command": "workbench.action.quickOpen", "when": "…" }`.
- A command with a `-` prefix removes a default binding.
- **`when` clause:**
  - Supports `!`, `&&`, `||`, `==`, `!=`, parentheses and context keys of boolean or string type.
  - Phase 1 context keys: `editorFocus`, `sideBarVisible`, `sideBarFocus`, `explorerViewletFocus`, `searchViewletFocus`, `inQuickOpen`, `findWidgetVisible`, `resourceExtname`, `editorLangId`.
- Supports chords, for example `cmd+k cmd+t`.
- The later rule with a matching `when` wins, like VS Code.
- Command IDs use VS Code's names when an equivalent command exists, so users can copy their old keybindings over.

### 5.4 Default shortcuts (macOS)

| Key | Command |
|---|---|
| Cmd+P | Quick Open |
| Cmd+Shift+P | Command palette |
| Ctrl+G | Go to line |
| Cmd+Shift+F | Search in project |
| Cmd+F | Find in file |
| Cmd+B | Hide/show sidebar |
| Cmd+Shift+E | Open Explorer |
| Cmd+\ | Split editor pane |
| Cmd+1…9 | Switch to pane N |
| Cmd+W | Close tab |
| Ctrl+Tab | Recent tabs |
| Cmd+K Cmd+T | Choose theme |
| Cmd+, | Open settings.json |
| Alt+Z | Toggle word wrap |
| Cmd+= / Cmd+- | Zoom text in / out |

## 6. Performance

Measured on a MacBook Pro M-series, release build.

| Metric | Target |
|---|---|
| From running `luc` to the first frame with content | < 200 ms |
| Opening a repo of about 80,000 files (Linux kernel) | File tree usable < 300 ms; scan finished < 5 s |
| Quick Open on 80,000 files, per keystroke | < 16 ms until results appear |
| Scrolling a 10 MB file | 120 fps, no dropped frames |
| Idle, average repo (about 5,000 files, 10 tabs) | < 150 MB RAM |
| File changes on disk → screen updates | < 100 ms |

## 7. Error handling

Principle: one error never brings down the whole editor.

| Situation | Handling |
|---|---|
| No read permission, file does not exist | The tab shows an error message and a "Retry" button |
| Binary file | The tab shows "Binary file" and an "Open as text" button |
| Unusual encoding | Guess with `encoding_rs`; the status bar shows the encoding in use |
| `settings.json` or `keybindings.json` has a syntax error | Keep the last valid configuration; the status bar reports the error with the line number; clicking it opens the file at that line |
| Theme broken or not found | Use One Dark, write a warning to the log |
| Tree-sitter parse fails or times out | Display without coloring, write to the log |
| Watcher queue overflow | Rescan everything (section 4.3) |
| Panic in a background task | `catch_unwind` catches it, cancels that task, writes to the log; the UI keeps running |
| Corrupt workspace state file | Skip it, use the default layout, rename the corrupt file to `.bak` |

Logging uses `tracing` with `tracing-appender`, rotated daily, kept for 7 days. The command "Lucerna: Open Logs Folder" is in the command palette.

## 8. Testing

- **Unit tests** for each crate without UI:
  - `text`: line, column and byte conversion; reload and changed-line computation; encoding detection.
  - `syntax`: language selection; highlights on a small sample per language (snapshot tests with `insta`).
  - `project`: gitignore filtering; fuzzy match scores; event merging.
  - `settings`: layer merging; per-language overrides; the `when` clause parser and evaluator; removing bindings with `-`; chords.
  - `theme`: reading One Dark and One Light; falling back by capture name.
- **Integration tests for `project`:**
  - Create a temp folder, then simulate an agent: create, edit, delete and rename files, and write 5,000 files in one batch.
  - Check that the snapshot and the emitted events are correct.
- **UI tests** with `gpui::TestAppContext`, run without a window: default layout, preview tab, pane splitting, Quick Open, saving and restoring state, keybindings with `when`.
- **Benchmarks** with `criterion`: large repo scan, fuzzy match, highlighting a 10 MB file, file reload. CI fails when a metric gets more than 15% slower.
- **CI** (GitHub Actions, running on macOS): `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`, `cargo deny check` (bans GPL, AGPL, LGPL), benchmarks.

## 9. Licensing

- **Lucerna source code:** MIT OR Apache-2.0.
- **Allowed dependencies:** MIT, Apache-2.0, BSD-2/3, ISC, Zlib, Unicode-3.0, MPL-2.0 (library form). Enforced by `deny.toml`.
- **From Zed, only the following may be used:**
  - Apache-2.0 crates: `gpui` and its foundation crates, plus their internal Apache dependencies.
  - The One theme (MIT), with the license file.
- **Grammars and queries:** taken from each grammar's original repo (mostly MIT). Listed in `THIRD_PARTY_NOTICES.md`.

## 10. Risks

| Risk | Mitigation |
|---|---|
| GPUI changes its API, no new release yet | Pin to a commit; update in a separate PR; keep all GPUI code inside `lucerna-workbench` |
| GPUI lacks widgets (tree, input, virtual list) | Write them on top of `uniform_list` and `div`; consult `gpui-component` after checking the license |
| Accidentally copying GPL code from Zed | `cargo-deny` in CI; a convention of not opening the source of GPL crates while writing the corresponding module |
| Text rendering or font fallback for Vietnamese and CJK is wrong | Put Vietnamese, CJK and emoji sample files into manual testing before release |
| Scanning a large repo uses a lot of RAM | Store paths as `Arc<str>` or share prefixes; measure in benchmarks |

## 11. Next steps

After this spec is approved, write the implementation plan with the `writing-plans` skill. Expected order:

1. Create the workspace and CI.
2. `settings` and `theme`.
3. `text`.
4. `project`.
5. `syntax`.
6. `workbench`: window frame, then explorer, then code view.
7. Quick Open and search.
8. Saving workspace state.
9. Benchmarks and tuning.

## 12. Change 2026-11-08: Zed-style UI

The user decided on a **modern Zed-style layout and look**, while **keeping VS Code-style shortcuts, `settings.json` and `keybindings.json`**. This section replaces the "Layout" part of section 2.1 and the part of "Design, part 1" about the activity bar.

**Layout**
- **No activity bar.** The sidebar (left dock) is toggled with Cmd+B.
- **Panel switch buttons in the status bar.** Two icon buttons, Explorer and Search, sit at the left corner of the status bar. Clicking the button of the open panel hides the sidebar; clicking the other button switches panels. Cmd+Shift+E and Cmd+Shift+F still work as before.
  - *Change 2026-11-10:* Search is no longer a panel in the sidebar but a tab in the editor. The magnifier button opens that tab. See `2026-11-10-project-search-tab-design.md`.
- **Integrated title bar:** 34 px tall, transparent background, the three macOS traffic-light buttons sit inside the window. To the right of the three buttons is the project's root folder name. Drag the title bar to move the window, double-click to zoom.
- **Status bar** 26 px tall. On the left are the panel switch buttons, on the right are info items (line/column, language, encoding, LF/CRLF).

**Style**
- **Tabs:** flat, 32 px tall. The selected tab has a background matching the editor background and no bottom border. An unselected tab uses `tab.inactive_background`. The close button only shows when hovering over the tab or on the selected tab. The border between tabs is 1 px thick, in the `border.variant` color.
- **Spacing on a 4 px grid.** Each row in the file tree is 24 px tall. The UI font is the system font, 13 px. The editor font follows `editor.fontFamily`.
- **Popups** (Quick Open, theme picker, search widget): float at the top center of the screen, at most 600 px wide, 8 px rounded corners, 1 px border, with a drop shadow, background `elevated_surface.background`.
- **Hover:** on mouse hover, the background switches to `ghost_element.hover`.
- **Panels have no heavy header.** Explorer only has one line with the root folder name, small text, in a faint color.
- **New colors to read from the Zed theme**, all with fallback values: `title_bar.background`, `elevated_surface.background`, `ghost_element.hover`, `border.variant`, `icon`, `icon.muted`.

## 13. Change 2026-11-09: UI refinement to be as compact as Zed

The user approved this section after comparing Lucerna screenshots with Zed. It replaces the corresponding points in sections 2.1, 5.2 and 12.

**Fonts**
- Lucerna bundles the **Lilex** font (Regular, Bold, Italic, BoldItalic) and **IBM Plex Sans** (Regular, Medium). The fonts come from their original repos, with the OFL-1.1 license, and are recorded in `THIRD_PARTY_NOTICES.md`.
- The fonts are registered with GPUI at startup.
- New default values:
  - `editor.fontFamily` = `"Lilex"`, `editor.fontSize` = 14, `editor.lineHeight` = 1.6.
  - The UI font is IBM Plex Sans at size 14. Tabs, the status bar and headers use size 13.

**Icons**
- All icons use the **Lucide** set (ISC license). The SVG files are copied into `crates/lucerna-workbench/assets/icons/` and embedded in the binary with `include_bytes!`.
- There are open and closed folder icons.
- File icons are chosen by extension or file name:

  | Icon | Applies to |
  |---|---|
  | file-code | source code files |
  | braces | json |
  | book-open | md |
  | settings | toml, yaml |
  | lock | `*.lock` |
  | git-branch | `.gitignore`, `.gitattributes` |
  | scale | `LICENSE*` |
  | terminal | sh, bash, zsh |
  | image | images |
  | file | default |

- The icon color is `icon.muted`.

**File tree**
- The first row is the root row, with a folder icon and the project folder name exactly as on disk (case not forced). This row replaces the uppercase header in section 12.
- Each indent level has a 1 px vertical guide line in the `border.variant` color.
- There is a thin scrollbar.

**Editor**
- `editor.minimap.enabled` defaults to `false`.
- The fold arrow in the gutter only shows when hovering over the gutter. A folded line always shows the arrow.
- There is a thin vertical scrollbar (about 6 px) on the right.
- Right below the tab bar there is a thin path bar, showing the relative path of the open file in the `text.muted` color.

**Tabs and title bar**
- Tab text is size 13, with tighter padding. An unselected tab uses the `text.muted` color.
- The selected tab of the focused editor group uses the `text` color. The selected tab of an unfocused group is a bit dimmer: a color between `text` and `text.muted`.
- The title bar shows the git branch name next to the project name, with the Lucide `git-branch` icon (not the ⎇ character): `lucerna [git-branch] main`. The branch name is read from `.git/HEAD` and also supports the case where `.git` is a file. The name updates itself when the branch changes and adds no git library. On a detached HEAD, the abbreviated SHA is shown.

**Not done**
- Showing gitignored files dimmed, because it requires changing how scanning works.
- Back/forward buttons, because they need navigation history.
