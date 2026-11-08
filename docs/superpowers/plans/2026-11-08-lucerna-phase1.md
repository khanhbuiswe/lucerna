# Lucerna Phase 1 (Core Viewer) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `luc`, a read-only, VS Code–style code viewer for macOS. It shows a project in a file tree and tabs, highlights code with tree-sitter, has Quick Open and project search, and reloads files the moment an agent changes them on disk.

**Architecture:**
- A Cargo workspace of seven crates.
- Five pure-Rust core crates: `text`, `syntax`, `project`, `settings`, `theme`. Each can be tested without a window.
- One UI crate, `lucerna-workbench`, is the only crate that depends on GPUI. GPUI is pinned to a git revision of the Zed repo.
- One binary crate, `lucerna`, which builds `luc`.
- Heavy work runs on background threads against immutable snapshots, and results are sent back to the UI thread.

**Tech Stack:**
- **Language:** Rust (stable, edition 2024).
- **UI:** `gpui` from the Zed repo (Apache-2.0).
- **Text:** `ropey`, `encoding_rs`, `chardetng`.
- **Syntax:** `tree-sitter` plus upstream grammar crates.
- **Files and search:** `ignore`, `notify`, `grep-searcher`, `grep-regex`, `nucleo-matcher`.
- **Config and parsing:** `serde`, `serde_json`, `jsonc-parser`.
- **CLI and logging:** `clap`, `tracing`, `tracing-appender`.
- **Testing:** `insta`, `criterion`, `tempfile`.
- **CI:** `cargo-deny`.

**Spec:** `docs/superpowers/specs/2026-11-08-lucerna-phase1-design.md`

## Global Constraints

- Product name: **Lucerna**. CLI binary: **`luc`**. Root crate: **`lucerna`**. Every crate is named `lucerna-*`.
- Lucerna's own license: **MIT OR Apache-2.0**.
- Allowed dependency licenses, enforced by `deny.toml`: MIT, Apache-2.0, BSD-2-Clause, BSD-3-Clause, ISC, Zlib, Unicode-3.0, MPL-2.0.
  - Banned: GPL, AGPL and LGPL of any version.
  - No GPL crate from the Zed repo, whether as a dependency or as copied code.
- Only `lucerna-workbench` may depend on `gpui`. `gpui` comes in as a git dependency pinned to a single commit (`rev = "…"`), chosen in Task 1.
- Platform: macOS 13 or newer, Apple Silicon and Intel. No Linux or Windows code paths in Phase 1.
- Read-only: Phase 1 has no text editing. `lucerna-text` still uses a rope.
- No telemetry and no network access anywhere.
- File paths:

  | What | Path |
  |---|---|
  | User settings | `~/.config/lucerna/settings.json` |
  | User keybindings | `~/.config/lucerna/keybindings.json` |
  | User themes | `~/.config/lucerna/themes/*.json` |
  | Workspace settings | `<root>/.lucerna/settings.json` |
  | Workspace state | `~/Library/Application Support/Lucerna/workspaces/<sha256(root)>.json` |
  | Logs | `~/Library/Logs/Lucerna/` |

- Limits:
  - Large file, set in settings: `lucerna.largeFile.maxBytes` = 20971520 (20 MB) and `lucerna.largeFile.maxLines` = 300000. Above either one: no highlighting, no minimap, no folding.
  - Hard open limit: 536870912 bytes (512 MB).
  - Long lines render only the first 10000 chars, followed by `…`.
- Timings:
  - Watcher debounce: 50 ms.
  - A batch of more than 1000 raw events, or a watcher overflow, triggers a full rescan.
  - The external-change decoration lasts 10 s.
  - Search debounce: 150 ms. The search result cap is 20000.
  - Quick Open returns 50 results.
  - Synchronous parse budget: 1 ms.
  - Highlighting covers the visible rows ± 100.
  - Tree scan batches arrive every 16 ms.
- Performance targets (release build, M-series Mac):

  | Metric | Target |
  |---|---|
  | First frame | < 200 ms |
  | 80k-file repo: tree usable | < 300 ms |
  | 80k-file repo: scan done | < 5 s |
  | Quick Open per keystroke | < 16 ms |
  | Scrolling a 10 MB file | 120 fps |
  | Idle RAM | < 150 MB (≈5k files, 10 tabs) |
  | Disk change → screen | < 100 ms |

- Command IDs use VS Code's names wherever an equivalent exists.

## Review Focus

1. **Atomic writes.** An agent may write a temp file and then rename it over the target. That must show up as a single `Modified` for the target: no "deleted" tab state, and no ghost temp file left in the tree. Test owner: Task 6.
2. **Line endings.** Files with CRLF, mixed endings or no trailing newline must count lines correctly, render without a stray `\r`, and keep scroll position correctly on reload. Test owner: Task 4.
3. **Symlink loops.** A project containing a symlink loop (`a -> .`) must finish scanning. Symlinks are listed but never followed. Test owner: Task 5.
4. **Minified files.** A minified single-line file (one line, 5 MB) must open without freezing. It renders truncated at 10000 chars, and highlighting is limited to the truncated window. Test owners: Tasks 9 and 14.
5. **File shrinks while open.** A file truncated to empty, or shortened below the current scroll position, must clamp the scroll and selection without panicking. Test owner: Task 4 (`Reload::adjust_row` clamps), and Task 15 checks that the view applies it.

---

### Task 1: Workspace scaffold, CI, license gate, GPUI pin

**Files:**
- Create: `Cargo.toml`, `rust-toolchain.toml`, `deny.toml`, `LICENSE-MIT`, `LICENSE-APACHE`, `THIRD_PARTY_NOTICES.md`, `.gitignore`, `.github/workflows/ci.yml`
- Create: `crates/lucerna/Cargo.toml`, `crates/lucerna/src/main.rs`
- Create: `crates/lucerna-workbench/Cargo.toml`, `crates/lucerna-workbench/src/lib.rs`, `crates/lucerna-workbench/examples/hello_window.rs`
- Test: `crates/lucerna/tests/cli.rs`

**Interfaces:**
- Produces:
  - Workspace members `crates/*`.
  - `[workspace.dependencies]` entries that every later task adds to.
  - A `gpui = { git = "https://github.com/zed-industries/zed", rev = "<sha>" }` entry. Add the other GPUI crates (`gpui_platform` etc.) at the same rev if that revision's gpui README requires them.
  - The `luc` binary.
  - `[profile.dev.package]` with `opt-level = 3` for `tree-sitter`, `taffy` and `ropey`.

- [ ] **Step 1: Write failing CLI test** `version_flag_prints_name_and_version`. Use `assert_cmd` to run `luc --version`, and assert that stdout equals `"luc 0.1.0\n"`.
- [ ] **Step 2: Run** `cargo test -p lucerna --test cli`. Expected: FAIL (no binary).
- [ ] **Step 3: Implement** `main` with `clap` derive.
  - `#[command(name = "luc", version)]`.
  - One optional positional `path: Option<PathBuf>`.
  - For now, `main` only parses the arguments.
- [ ] **Step 4: Run** `cargo test -p lucerna --test cli`. Expected: PASS.
- [ ] **Step 5: Pin GPUI.**
  1. Pick the newest `main` commit of `zed-industries/zed` whose `crates/gpui/LICENSE-APACHE` exists.
  2. Add it to the workspace dependencies.
  3. Write `examples/hello_window.rs`, which opens an 800×600 window showing the text "Lucerna".
- [ ] **Step 6: Run** `cargo run -p lucerna-workbench --example hello_window`. Expected: a window opens with the text. Close it.
- [ ] **Step 7: Write `deny.toml`.**
  - `[licenses] allow = [...]`, set exactly to the Global Constraints list.
  - `[bans]` with `multiple-versions = "warn"`.
  - `[sources]` allowing crates.io and the one Zed git URL.
- [ ] **Step 8: Run** `cargo deny check licenses`. Expected: `licenses ok`.
  - If a gpui transitive dependency uses a license outside the list, add it **only** if it is permissive, and record it in `THIRD_PARTY_NOTICES.md`.
- [ ] **Step 9: Write `ci.yml`** to run on `macos-14`: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, and `cargo deny check`.
- [ ] **Step 10: Commit** with message `chore: scaffold workspace, CI, license gate, pin gpui`.

### Task 2: Layered settings (`lucerna-settings`)

**Files:**
- Create: `crates/lucerna-settings/Cargo.toml`, `src/lib.rs`, `src/store.rs`, `src/error.rs`, `assets/default-settings.json`
- Test: `crates/lucerna-settings/tests/store.rs`

**Interfaces:**
- Produces:
  - `pub enum Layer { Default, User, Workspace }`
  - `pub struct SettingsError { pub message: String, pub line: Option<usize> }`. `line` is 1-based.
  - `pub struct SettingsStore`, with these methods:
    - `SettingsStore::with_defaults() -> SettingsStore`. Loads `default-settings.json` through `include_str!`.
    - `set_layer(&mut self, layer: Layer, jsonc: &str) -> Result<(), SettingsError>`. On error, the layer's previous value is kept.
    - `get<T: serde::de::DeserializeOwned>(&self, key: &str, language: Option<&str>) -> Option<T>`.
- Key lookup rules:
  - In each layer, first try the exact flat key (`"editor.fontSize": 14`), then try the nested path (`{"editor": {"fontSize": 14}}`).
  - Precedence, highest first: Workspace `[lang]`, Workspace, User `[lang]`, User, Default `[lang]`, Default.
  - A key found in a higher layer **replaces** the whole lower value. Object values are not merged.
- `default-settings.json` holds exactly these keys:

  | Key | Value |
  |---|---|
  | `editor.fontFamily` | `"Menlo"` |
  | `editor.fontSize` | `13` |
  | `editor.lineHeight` | `1.5` |
  | `editor.wordWrap` | `"off"` |
  | `editor.minimap.enabled` | `true` |
  | `editor.renderWhitespace` | `"none"` |
  | `editor.tabSize` | `4` |
  | `files.exclude` | `{"**/.git": true, "**/.DS_Store": true}` |
  | `search.exclude` | `{"**/node_modules": true, "**/target": true}` |
  | `workbench.colorTheme` | `"One Dark"` |
  | `workbench.editor.enablePreview` | `true` |
  | `explorer.autoReveal` | `true` |
  | `explorer.decorations.externalChanges` | `true` |
  | `lucerna.largeFile.maxBytes` | `20971520` |
  | `lucerna.largeFile.maxLines` | `300000` |

- [ ] **Step 1: Write failing tests.**
  - `defaults_present`: `get::<u32>("editor.fontSize", None) == Some(13)`.
  - `user_overrides_default`: User `{"editor.fontSize": 15}` gives `15`.
  - `workspace_overrides_user`: User sets `15` and Workspace sets `16`; the result is `16`.
  - `nested_form_supported`: User `{"editor": {"tabSize": 2}}` gives `2`.
  - `language_override`: User `{"[rust]": {"editor.tabSize": 2}}` gives `2` for `Some("rust")` and `4` for `None`.
  - `objects_replace_not_merge`: User `{"files.exclude": {"**/dist": true}}` gives a map with exactly one key.
  - `comments_and_trailing_commas_ok`: `"// c\n{\"editor.fontSize\": 14,}"` parses.
  - `invalid_json_keeps_previous_layer_and_reports_line`:
    1. Set User to `{"editor.fontSize": 15}`.
    2. Then set User to `"{\n\"a\": 1\n\"b\": 2\n}"`. This returns `Err` with `line == Some(3)`.
    3. `get("editor.fontSize")` is still `15`.
- [ ] **Step 2: Run** `cargo test -p lucerna-settings`. Expected: FAIL.
- [ ] **Step 3: Implement** `SettingsStore`.
  - Parse with `jsonc_parser::parse_to_serde_value` with comments and trailing commas allowed.
  - Convert the error range to a line by counting `\n` before the error's start offset.
- [ ] **Step 4: Run** `cargo test -p lucerna-settings`. Expected: PASS.
- [ ] **Step 5: Commit** with message `feat(settings): layered JSONC settings store`.

### Task 3: Keybindings and when-clauses (`lucerna-settings`)

**Files:**
- Create: `crates/lucerna-settings/src/when.rs`, `src/keymap.rs`, `assets/default-keybindings.json`
- Test: `crates/lucerna-settings/tests/when.rs`, `tests/keymap.rs`

**Interfaces:**
- Consumes: `SettingsError` (Task 2).
- Produces:
  - **Context values:**
    - `pub enum ContextValue { Bool(bool), Str(String) }`
    - `pub struct Context(HashMap<String, ContextValue>)`, with `set(&mut self, key, value)`.
  - **When-clauses:**
    - `pub enum WhenExpr { … }`
    - `pub fn parse_when(src: &str) -> Result<WhenExpr, SettingsError>`
    - `WhenExpr::eval(&self, ctx: &Context) -> bool`
  - **Keystrokes:**
    - `pub struct Keystroke { pub cmd: bool, pub ctrl: bool, pub alt: bool, pub shift: bool, pub key: String }`. `key` is lowercase, for example `"p"`, `"\\"`, `"tab"`, `"="`.
    - `pub fn parse_keys(s: &str) -> Result<Vec<Keystroke>, SettingsError>`. A space separates the parts of a chord.
  - **Bindings:**
    - `pub struct KeyBinding { pub keys: Vec<Keystroke>, pub command: String, pub args: Option<serde_json::Value>, pub when: Option<WhenExpr> }`
  - **Keymap:**
    - `pub struct Keymap`
    - `Keymap::load(default_json: &str, user_json: Option<&str>) -> (Keymap, Option<SettingsError>)`. A broken user file yields the defaults plus the error.
    - `Keymap::default_bindings() -> &'static str`, which is the asset.
    - `Keymap::resolve(&self, pending: &[Keystroke], ctx: &Context) -> Resolution`
    - `pub enum Resolution { Command { command: String, args: Option<serde_json::Value> }, Pending, NoMatch }`
    - `Keymap::label_for(&self, command: &str) -> Option<String>`. Returns the label of the last-defined binding, for example `"⌘P"` or `"⌘K ⌘T"`.
- When-clause grammar:
  - Operators: `!`, `&&`, `||`, `==`, `!=` and parentheses. `&&` binds tighter than `||`.
  - The right-hand side of `==`/`!=` is a single-quoted string or a bare word.
  - An unknown key is falsy and compares as `""`.
- Resolution:
  - Bindings are scanned from last to first; the first whose `keys` match fully and whose `when` is true or absent wins.
  - If no full match exists and `pending` is a strict prefix of some active binding, the result is `Pending`.
  - A user entry whose `command` starts with `-` removes the earlier bindings that have the same `keys` and the command without the `-`. If the entry also has a `when`, a binding is removed only when its `when` source text is equal.
- `default-keybindings.json`:

  | Keys | Command |
  |---|---|
  | `cmd+p` | `workbench.action.quickOpen` |
  | `cmd+shift+p` | `workbench.action.showCommands` |
  | `ctrl+g` | `workbench.action.gotoLine` |
  | `cmd+shift+f` | `workbench.action.findInFiles` |
  | `cmd+f` (`when: editorFocus`) | `actions.find` |
  | `escape` (`when: findWidgetVisible`) | `closeFindWidget` |
  | `escape` (`when: inQuickOpen`) | `workbench.action.closeQuickOpen` |
  | `cmd+b` | `workbench.action.toggleSidebarVisibility` |
  | `cmd+shift+e` | `workbench.view.explorer` |
  | `cmd+\` | `workbench.action.splitEditor` |
  | `cmd+1` … `cmd+8` | `workbench.action.focusFirstEditorGroup` … `focusEighthEditorGroup` |
  | `cmd+w` | `workbench.action.closeActiveEditor` |
  | `ctrl+tab` | `workbench.action.quickOpenPreviousRecentlyUsedEditorInGroup` |
  | `cmd+k cmd+t` | `workbench.action.selectTheme` |
  | `cmd+,` | `workbench.action.openSettingsJson` |
  | `alt+z` | `editor.action.toggleWordWrap` |
  | `cmd+=` | `editor.action.fontZoomIn` |
  | `cmd+-` | `editor.action.fontZoomOut` |
  | `cmd+c` (`when: editorFocus`) | `editor.action.clipboardCopyAction` |

  Cmd+9 is left out, to match VS Code, which has only eight group-focus commands.

- [ ] **Step 1: Write failing tests** in `tests/when.rs`.
  - `bare_key_truthy`: `editorFocus` is true when it is `Bool(true)`, and false when it is unset.
  - `not_and_or_precedence`: `"a || b && c"` with a=false, b=true, c=false gives false.
  - `parens`: `"(a || b) && c"`.
  - `string_equality`: `"resourceExtname == '.rs'"` and `"editorLangId != rust"`.
  - `parse_error_reports`: `"a &&"` gives `Err`.
- [ ] **Step 2: Write failing tests** in `tests/keymap.rs`.
  - `parses_modifiers`: `"cmd+shift+p"`.
  - `parses_chord`: `"cmd+k cmd+t"` gives a Vec of length 2.
  - `default_cmd_p_resolves`: resolving `cmd+p` returns `workbench.action.quickOpen`.
  - `chord_pending_then_command`.
  - `when_gates_binding`: `escape` resolves to `closeFindWidget` only when `findWidgetVisible` is true.
  - `later_binding_wins`: a user binding `cmd+p → foo` beats the default.
  - `minus_removes_default`: a user entry `{"key":"cmd+b","command":"-workbench.action.toggleSidebarVisibility"}` makes `cmd+b` resolve to `NoMatch`.
  - `broken_user_file_falls_back`: user `"["` gives the default keymap and `Some(err)`.
  - `label_for_quick_open`: `label_for("workbench.action.quickOpen") == Some("⌘P")`.
- [ ] **Step 3: Run** `cargo test -p lucerna-settings`. Expected: FAIL.
- [ ] **Step 4: Implement** `when.rs` (recursive descent parser) and `keymap.rs`.
- [ ] **Step 5: Run** `cargo test -p lucerna-settings`. Expected: PASS.
- [ ] **Step 6: Commit** with message `feat(settings): VS Code–compatible keymap with when-clauses`.

### Task 4: Rope buffer (`lucerna-text`)

**Files:**
- Create: `crates/lucerna-text/Cargo.toml`, `src/lib.rs`, `src/buffer.rs`, `src/load.rs`, `src/reload.rs`
- Test: `crates/lucerna-text/tests/buffer.rs`

**Interfaces:**
- Produces:
  - `pub const MAX_OPEN_BYTES: u64 = 536_870_912;`
  - `pub enum LoadError { NotFound, PermissionDenied, Binary, TooLarge { bytes: u64 }, Io(String) }`
  - `pub enum LineEnding { Lf, CrLf }`. This is the dominant ending, used for display in the status bar.
  - `pub struct Point { pub row: u32, pub column: u32 }`. `column` is a UTF-8 byte offset within the line.
  - **`Buffer`:**
    - `Buffer::load(path: &Path) -> Result<Buffer, LoadError>`
    - `Buffer::from_bytes(bytes: &[u8], force_text: bool) -> Result<Buffer, LoadError>`
    - `encoding(&self) -> &'static encoding_rs::Encoding`
    - `line_ending(&self) -> LineEnding`
    - `snapshot(&self) -> BufferSnapshot`
    - `reload(&mut self, bytes: &[u8]) -> Result<Reload, LoadError>`
    - `is_large(&self, max_bytes: u64, max_lines: u32) -> bool`
  - **`BufferSnapshot`** (`Clone`, cheap):
    - `line_count(&self) -> u32`. An empty file has 1 line, and a file ending in `\n` does not count an extra empty last line.
    - `line(&self, row: u32) -> std::borrow::Cow<'_, str>`. Returned without `\n` or `\r\n`.
    - `len_bytes(&self) -> usize`
    - `point_to_offset(&self, p: Point) -> usize`
    - `offset_to_point(&self, offset: usize) -> Point`
    - `text_for_range(&self, r: Range<usize>) -> String`
    - `chunks_in_range(&self, r: Range<usize>) -> impl Iterator<Item = &str>`, used to feed tree-sitter.
  - **`Reload`:**
    - `pub struct Reload { pub start_row: u32, pub old_end_row: u32, pub new_end_row: u32 }` (end rows are exclusive)
    - `Reload::adjust_row(&self, row: u32, new_line_count: u32) -> u32`
- Load decisions:
  - **Size first:** more than `MAX_OPEN_BYTES` gives `TooLarge`.
  - **Binary:** a NUL in the first 8192 bytes, unless `force_text`, gives `Binary`.
  - **Encoding**, in this order: a BOM if present, then UTF-8 if valid, otherwise `chardetng` with `guess(None, true)`. Decode with `encoding_rs`.
  - `NotFound` and `PermissionDenied` come from the io error kind.
- Reload diff: compare old and new by lines. `start_row` is the first row that differs (from the common prefix). The ends come from the common suffix.
- `adjust_row` rules, in order:
  1. A row before `start_row` stays the same.
  2. A row at or after `old_end_row` becomes `row - old_end_row + new_end_row`.
  3. Any other row becomes `start_row`.
  4. Finally, clamp the result to `new_line_count.saturating_sub(1)`.

- [ ] **Step 1: Write failing tests.**
  - `utf8_lines`: `"a\nb\n"` has `line_count == 2`; `line(1) == "b"`.
  - `empty_file_one_line`.
  - `crlf_stripped`: `"a\r\nb"` gives `line(0) == "a"` and `line_ending == CrLf`.
  - `mixed_endings_and_no_trailing_newline`: `"a\r\nb\nc"` gives 3 lines, and no line contains `\r`.
  - `point_offset_roundtrip_multibyte`: `"xin chào\nđ"`, round-tripping every char boundary.
  - `binary_detected`: `b"ab\0cd"` gives `Err(Binary)`; with `force_text=true` it gives `Ok`.
  - `utf16_bom_decoded`: UTF-16LE BOM bytes of `"hi"` give `line(0) == "hi"`.
  - `latin1_guessed`: the bytes `b"caf\xe9 cr\xe8me"` decode to a string containing `"café"`.
  - `too_large`: use `from_bytes` with a fake length via a helper `check_size(len: u64)` returning `TooLarge`. Do not allocate 512 MB.
  - `reload_insert_above_shifts_row`: old has 10 lines, new inserts 3 lines at row 2. Then `adjust_row(7, 13) == 10` and `adjust_row(1, 13) == 1`.
  - `reload_truncate_to_empty_clamps`: old has 100 lines, new is `""`. Then `adjust_row(80, 1) == 0`.
  - `reload_edit_inside_changed_region`: a row inside the changed region maps to `start_row`.
- [ ] **Step 2: Run** `cargo test -p lucerna-text`. Expected: FAIL.
- [ ] **Step 3: Implement** `load.rs`, `buffer.rs` and `reload.rs` using `ropey::Rope`. `BufferSnapshot` wraps a `Rope` clone.
- [ ] **Step 4: Run** `cargo test -p lucerna-text`. Expected: PASS.
- [ ] **Step 5: Commit** with message `feat(text): rope buffer with encoding detection and reload diff`.

### Task 5: Worktree scan (`lucerna-project`)

**Files:**
- Create: `crates/lucerna-project/Cargo.toml`, `src/lib.rs`, `src/worktree.rs`, `src/snapshot.rs`
- Test: `crates/lucerna-project/tests/scan.rs`

**Interfaces:**
- Produces:
  - `pub enum EntryKind { File, Dir, Symlink }`
  - `pub struct Entry { pub path: Arc<Path>, pub kind: EntryKind }`. `path` is relative to the root.
  - `pub struct WorktreeSnapshot`, with:
    - `entries(&self) -> &[Entry]`. Sorted depth-first: directories before files at each level, then case-insensitive by name.
    - `files(&self) -> impl Iterator<Item=&Entry>`
    - `children(&self, dir: &Path) -> &[Entry]`
    - `contains(&self, path: &Path) -> bool`
    - `scan_complete(&self) -> bool`
  - `pub struct ScanOptions { pub exclude: Vec<String> }`. These are glob strings taken from the `files.exclude` keys whose value is `true`.
  - `pub enum ChangeKind { Added, Removed, Modified }`
  - `pub struct Change { pub path: Arc<Path>, pub kind: ChangeKind }`
  - `pub enum WorktreeEvent { ScanProgress, ScanComplete, Changed(Vec<Change>), Rescanned }`
  - `pub struct Worktree`, with:
    - `Worktree::start(root: PathBuf, opts: ScanOptions) -> Worktree`. Spawns a background thread.
    - `root(&self) -> &Path`
    - `snapshot(&self) -> WorktreeSnapshot`, which clones an `Arc`.
    - `events(&self) -> crossbeam_channel::Receiver<WorktreeEvent>`
    - `set_options(&self, opts: ScanOptions)`, which triggers a rescan.
- Scan rules:
  - Use `ignore::WalkBuilder` with `hidden(false)`, `git_ignore(true)`, `require_git(false)` and `follow_links(false)`.
  - Exclude globs are applied with `ignore::overrides::OverrideBuilder`, each glob added as `!glob`.
  - Publish a new snapshot and send `ScanProgress` at most every 16 ms. Send `ScanComplete` at the end.

- [ ] **Step 1: Write failing tests** with `tempfile::TempDir` and a helper `wait_complete(&Worktree)`, which waits for `ScanComplete` with a 5 s timeout.
  - `lists_files_and_dirs_sorted`
  - `respects_gitignore_without_git_dir`: a `.gitignore` containing `out/` hides `out/x`.
  - `hidden_files_shown_but_files_exclude_applies`: `.env` is shown; `.git/` is hidden through `**/.git`.
  - `symlink_loop_terminates`: `a -> .`. The scan completes, and `a` is listed as `Symlink` with no children.
  - `unicode_and_space_paths`: `"thư mục/tệp có dấu.rs"` is listed.
  - `set_options_rescans`: adding exclude `**/*.log` removes `x.log` from the next snapshot after `Rescanned`.
- [ ] **Step 2: Run** `cargo test -p lucerna-project --test scan`. Expected: FAIL.
- [ ] **Step 3: Implement** `worktree.rs` and `snapshot.rs`. The snapshot is `Arc<Vec<Entry>>`, plus a `HashMap<Arc<Path>, Range<usize>>` child index rebuilt for each publish.
- [ ] **Step 4: Run** `cargo test -p lucerna-project --test scan`. Expected: PASS.
- [ ] **Step 5: Commit** with message `feat(project): gitignore-aware background worktree scan`.

### Task 6: File watching and coalescing (`lucerna-project`)

**Files:**
- Create: `crates/lucerna-project/src/watch.rs`, `src/coalesce.rs`
- Modify: `crates/lucerna-project/src/worktree.rs`, to start the watcher after the scan completes
- Test: `crates/lucerna-project/tests/coalesce.rs`, `tests/watch.rs`

**Interfaces:**
- Consumes: `Change`, `ChangeKind`, `WorktreeEvent` (Task 5).
- Produces:
  - `pub enum RawEvent { Created(PathBuf), Removed(PathBuf), Modified(PathBuf), Renamed { from: PathBuf, to: PathBuf }, Overflow }`
  - `pub enum Coalesced { Changes(Vec<Change>), Rescan }`
  - `pub fn coalesce(events: Vec<RawEvent>, root: &Path) -> Coalesced`
  - `pub const DEBOUNCE: Duration = Duration::from_millis(50);`
  - `pub const RESCAN_THRESHOLD: usize = 1000;`
- Coalescing rules:
  - Any `Overflow`, or `events.len() > RESCAN_THRESHOLD`, gives `Rescan`.
  - `Renamed` becomes Removed(from) followed by Added(to).
  - Per path, in order:

    | Sequence for one path | Result |
    |---|---|
    | Added, then Removed | dropped |
    | Removed, then Added | Modified |
    | Added, then Modified | Added |
    | Modified, then Removed | Removed |
    | anything else | last kind wins |

  - Paths are made relative to the root. Paths outside the root are dropped.
- Worktree side: ignored paths (`.gitignore`, exclude globs) are filtered out before a `Changed` event is sent. `Rescan` runs a full scan and then sends `Rescanned`.
- The watcher uses `notify::RecommendedWatcher` (FSEvents) and a debounce loop:
  1. Block on the first event.
  2. Collect events until 50 ms pass with no new event, or 100 ms total have elapsed.
  3. Call `coalesce`.

- [ ] **Step 1: Write failing unit tests** in `tests/coalesce.rs`.
  - `added_then_removed_dropped`
  - `removed_then_added_is_modified`
  - `atomic_write_via_rename`: Created(`f.tmp`), Modified(`f.tmp`), Renamed{`f.tmp`→`f`}. The result is exactly `[Modified(f)]`.
  - `over_threshold_rescans`: 1001 events give `Rescan`.
  - `overflow_rescans`
  - `outside_root_dropped`
- [ ] **Step 2: Write failing integration tests** in `tests/watch.rs`. Each waits on `events()` with a 2 s timeout.
  - `create_modify_delete_reported`
  - `atomic_rename_reports_modified_only`: no `Added`/`Removed` for `f.tmp`, and none for `f`.
  - `gitignored_changes_silent`: writing to `out/x` when `out/` is ignored sends no `Changed` event within 300 ms.
  - `bulk_write_5000_files_rescans`: the next event is `Rescanned`, and the snapshot contains all 5000 files.
- [ ] **Step 3: Run** `cargo test -p lucerna-project`. Expected: FAIL.
- [ ] **Step 4: Implement** `coalesce.rs`, then `watch.rs`, then wire them into `Worktree`.
- [ ] **Step 5: Run** `cargo test -p lucerna-project`. Expected: PASS. Run the watcher tests 5 times with `--test-threads=1` to check for flakiness.
- [ ] **Step 6: Commit** with message `feat(project): debounced FSEvents watcher with atomic-write coalescing`.

### Task 7: Fuzzy file finder (`lucerna-project`)

**Files:**
- Create: `crates/lucerna-project/src/fuzzy.rs`
- Test: `crates/lucerna-project/tests/fuzzy.rs`

**Interfaces:**
- Consumes: `WorktreeSnapshot` (Task 5).
- Produces:
  - `pub struct FileMatch { pub path: Arc<Path>, pub score: u32, pub positions: Vec<u32> }`. `positions` are char indices into the path string.
  - `pub fn match_files(snapshot: &WorktreeSnapshot, query: &str, recent: &[Arc<Path>], limit: usize, cancel: &AtomicBool) -> Vec<FileMatch>`
  - `pub const QUICK_OPEN_LIMIT: usize = 50;`
- Ordering:
  - **Empty query:** `recent` files in the order given (only those that still exist), then the remaining files sorted by path, up to `limit`.
  - **Non-empty query:** use `nucleo_matcher` with `Config::DEFAULT.match_paths()`, smart case. Sort by:
    1. Whether the file is in `recent` (recent first, keeping their order among themselves).
    2. Score, descending.
    3. Path length, ascending.
  - When `cancel` becomes true, return an empty Vec.

- [ ] **Step 1: Write failing tests.**
  - `matches_subsequence`: `"wbmain"` finds `crates/workbench/src/main.rs`.
  - `recent_first`
  - `shorter_path_breaks_tie`
  - `empty_query_recent_then_alpha`
  - `limit_respected`
  - `cancel_returns_empty`
  - `positions_point_at_matched_chars`
- [ ] **Step 2: Run** `cargo test -p lucerna-project --test fuzzy`. Expected: FAIL.
- [ ] **Step 3: Implement** `match_files`.
- [ ] **Step 4: Run** `cargo test -p lucerna-project --test fuzzy`. Expected: PASS.
- [ ] **Step 5: Commit** with message `feat(project): nucleo-based fuzzy file matching`.

### Task 8: Project content search (`lucerna-project`)

**Files:**
- Create: `crates/lucerna-project/src/search.rs`
- Test: `crates/lucerna-project/tests/search.rs`

**Interfaces:**
- Produces:
  - `pub struct SearchQuery { pub pattern: String, pub is_regex: bool, pub case_sensitive: bool, pub whole_word: bool, pub include: Vec<String>, pub exclude: Vec<String> }`
  - `pub struct LineMatch { pub line_number: u32, pub line_text: String, pub ranges: Vec<Range<usize>> }`
    - `line_number` is 1-based.
    - `ranges` are byte ranges into `line_text`.
    - `line_text` is truncated to 1000 chars.
  - `pub enum SearchEvent { File { path: Arc<Path>, matches: Vec<LineMatch> }, Done { total: usize, truncated: bool }, Error(String) }`
  - `pub const MAX_RESULTS: usize = 20_000;`
  - `pub fn search(root: &Path, query: &SearchQuery, settings_exclude: &[String], sink: crossbeam_channel::Sender<SearchEvent>, cancel: Arc<AtomicBool>)`
    - Blocking; the caller runs it on a thread.
    - Sends exactly one `Done` unless cancelled. A cancelled search sends nothing more.
- Rules:
  - The walk is `ignore::WalkBuilder::build_parallel()`, with the same ignore settings as Task 5.
  - The overrides contain `include` globs as positive patterns, plus `exclude` and `settings_exclude` as `!` patterns.
  - A literal pattern is escaped with `regex::escape`. A whole-word pattern is wrapped as `\b(?:…)\b`.
  - Build with `grep_regex::RegexMatcherBuilder`, with `case_insensitive(!case_sensitive)`.
  - Binary detection is `BinaryDetection::quit(0)`.
  - Stop once `MAX_RESULTS` matches have been sent, and report `truncated: true`.
  - An invalid regex sends `Error(msg)` and then `Done{0,false}`.

- [ ] **Step 1: Write failing tests.**
  - `finds_literal_with_ranges`
  - `regex_mode`
  - `case_sensitivity`
  - `whole_word`: `"log"` does not match `"login"`.
  - `include_glob_limits`
  - `settings_exclude_applied`
  - `gitignored_skipped`
  - `binary_skipped`
  - `cap_truncates`: 25000 matches give `total == 20000` and `truncated`.
  - `invalid_regex_reports_error`
  - `cancel_stops_without_done`
- [ ] **Step 2: Run** `cargo test -p lucerna-project --test search`. Expected: FAIL.
- [ ] **Step 3: Implement** `search`.
- [ ] **Step 4: Run** `cargo test -p lucerna-project --test search`. Expected: PASS.
- [ ] **Step 5: Commit** with message `feat(project): parallel ripgrep-core project search`.

### Task 9: Languages and highlighting (`lucerna-syntax`)

**Files:**
- Create: `crates/lucerna-syntax/Cargo.toml`, `src/lib.rs`, `src/registry.rs`, `src/syntax_state.rs`, `src/folds.rs`, `src/brackets.rs`
- Create: `crates/lucerna-syntax/tests/fixtures/*` (one small sample per language)
- Test: `crates/lucerna-syntax/tests/highlight.rs`, `tests/folds.rs`, `tests/brackets.rs`
- Modify: `THIRD_PARTY_NOTICES.md` (the grammar licenses)

**Interfaces:**
- Consumes: `BufferSnapshot`, `Point` (Task 4).
- Produces:
  - **Language registry:**
    - `pub struct Language { pub id: &'static str, pub name: &'static str, … }`
    - `pub struct LanguageRegistry`
    - `LanguageRegistry::bundled() -> LanguageRegistry`
    - `language_for_path(&self, path: &Path) -> Option<Arc<Language>>`
  - **Highlighting:**
    - `pub struct HighlightSpan { pub range: Range<usize>, pub capture: Arc<str> }`. The range is in absolute bytes.
    - `pub enum ParseOutcome { Done, TimedOut }`
    - `pub struct SyntaxState`, with:
      - `SyntaxState::new(lang: Arc<Language>) -> SyntaxState`
      - `parse(&mut self, snapshot: &BufferSnapshot, budget: Option<Duration>) -> ParseOutcome`. `None` means no time limit; `TimedOut` leaves the previous tree in place.
      - `highlights(&self, snapshot: &BufferSnapshot, rows: Range<u32>) -> Vec<HighlightSpan>`. Returns an empty Vec when there is no tree yet.
  - **Folds:** `pub fn indent_folds(snapshot: &BufferSnapshot, tab_size: u32, rows: Range<u32>) -> Vec<Range<u32>>`. Each range starts at the header row and has an exclusive end.
  - **Brackets:** `pub fn matching_bracket(snapshot: &BufferSnapshot, offset: usize) -> Option<usize>`
  - `pub const SYNC_PARSE_BUDGET: Duration = Duration::from_millis(1);`
- Bundled languages. Each takes its `HIGHLIGHTS_QUERY` (and `INJECTIONS_QUERY` where present) from its crate. When a crate exposes no query constant, `include_str!` the query from the grammar's own repository at the crate's version, with the license noted.

  | Language id | Name | Grammar crate | Files |
  |---|---|---|---|
  | `rust` | Rust | `tree-sitter-rust` | `.rs` |
  | `typescript` | TypeScript | `tree-sitter-typescript` (`LANGUAGE_TYPESCRIPT`) | `.ts` `.mts` `.cts` |
  | `typescriptreact` | TypeScript JSX | `tree-sitter-typescript` (`LANGUAGE_TSX`) | `.tsx` |
  | `javascript` | JavaScript | `tree-sitter-javascript` | `.js` `.mjs` `.cjs` `.jsx` |
  | `python` | Python | `tree-sitter-python` | `.py` `.pyi` |
  | `go` | Go | `tree-sitter-go` | `.go` |
  | `json` | JSON | `tree-sitter-json` | `.json` `.jsonc`, and filenames `tsconfig.json`, `settings.json` |
  | `markdown` | Markdown | `tree-sitter-md` (block + inline) | `.md` `.markdown` |
  | `toml` | TOML | `tree-sitter-toml-ng` | `.toml`, and `Cargo.lock` |
  | `yaml` | YAML | `tree-sitter-yaml` | `.yml` `.yaml` |
  | `shellscript` | Shell | `tree-sitter-bash` | `.sh` `.bash` `.zsh`, and `.zshrc` `.bashrc` |
  | `html` | HTML | `tree-sitter-html` | `.html` `.htm` |
  | `css` | CSS | `tree-sitter-css` | `.css` |

- Injection support in Phase 1 is **only** Markdown → markdown-inline. For each `inline` node that intersects the requested rows, parse it with the inline grammar and offset its spans. Other injection queries are ignored.
- `highlights` uses `QueryCursor::set_byte_range` over the requested rows. A line longer than 10000 chars contributes spans only for its first 10000 chars.
- Brackets: `()[]{}`. If the character at `offset` (or just before it) is a bracket whose node in the tree is a leaf, find the paired leaf among the siblings of its parent node. With no tree, scan the text with a depth counter, giving up after 1 MB.

- [ ] **Step 1: Write failing tests.**
  - `detects_by_extension_and_filename`
  - `unknown_extension_none`
  - `rust_keyword_highlighted`: in `"fn main() {}"`, a span covering `fn` has a capture starting with `"keyword"`.
  - `every_bundled_language_parses_fixture`: for each language, the fixture parses with no `ERROR` node at the root and gives at least one span.
  - `markdown_inline_emphasis_highlighted`: in `"# T\n\n**bold**"`, some span lies inside the `bold` bytes.
  - `highlights_limited_to_rows`: no span starts after the last requested row.
  - `minified_line_spans_capped`: a single-line 5 MB JS file gives `parse(…, None)` returning `Done`, and every span for row 0 ends at or before byte 10000.
  - `timeout_keeps_old_tree`: `parse` with a `Duration::ZERO` budget on a large input returns `TimedOut`, and `highlights` from the earlier tree is still non-empty.
  - Snapshot tests: `insta::assert_debug_snapshot!` of the Rust fixture's spans.
  - `indent_folds_python_and_braces`
  - `matching_bracket_nested`
  - `matching_bracket_ignores_bracket_in_string`, for Rust `"(\")\")"`.
- [ ] **Step 2: Run** `cargo test -p lucerna-syntax`. Expected: FAIL.
- [ ] **Step 3: Implement** the registry, then `SyntaxState`, the folds and the brackets.
  - The time budget uses tree-sitter's parse progress callback, or the timeout API, whichever the pinned version provides.
  - The parse reads text through `snapshot.chunks_in_range`.
- [ ] **Step 4: Run** `cargo test -p lucerna-syntax`. Expected: PASS. Run `cargo deny check licenses`. Expected: ok.
- [ ] **Step 5: Commit** with message `feat(syntax): bundled tree-sitter languages, viewport highlights, folds, brackets`.

### Task 10: Zed theme loading (`lucerna-theme`)

**Files:**
- Create: `crates/lucerna-theme/Cargo.toml`, `src/lib.rs`, `src/schema.rs`, `src/registry.rs`
- Create: `crates/lucerna-theme/assets/one.json` and `assets/LICENSE-one`. Both are copied from Zed's `assets/themes/one/` (MIT).
- Test: `crates/lucerna-theme/tests/theme.rs`

**Interfaces:**
- Produces:
  - `pub struct Rgba { pub r: f32, pub g: f32, pub b: f32, pub a: f32 }`, built with `Rgba::parse_hex(&str) -> Option<Rgba>`. Accepts `#rrggbb` and `#rrggbbaa`.
  - `pub enum Appearance { Light, Dark }`
  - `pub struct HighlightStyle { pub color: Option<Rgba>, pub italic: bool, pub bold: bool }`
  - `pub struct UiColors`. Every field is `Rgba`, with a fallback when the theme lacks the key:

    | Field | Zed key |
    |---|---|
    | `background` | `background` |
    | `border` | `border` |
    | `text` | `text` |
    | `text_muted` | `text.muted` |
    | `element_selected` | `element.selected` |
    | `panel_background` | `panel.background` |
    | `status_bar_background` | `status_bar.background` |
    | `tab_active_background` | `tab.active_background` |
    | `tab_inactive_background` | `tab.inactive_background` |
    | `editor_background` | `editor.background` |
    | `editor_foreground` | `editor.foreground` |
    | `editor_line_number` | `editor.line_number` |
    | `editor_active_line_number` | `editor.active_line_number` |
    | `editor_active_line_background` | `editor.active_line.background` |
    | `search_match_background` | `search.match_background` |
    | `selection` | `players[0].selection` |

  - `pub struct Theme { pub name: String, pub appearance: Appearance, pub ui: UiColors, … }`
  - `Theme::style_for(&self, capture: &str) -> Option<&HighlightStyle>`. Exact match first, then repeatedly drop the last `.segment`.
  - **Theme registry:**
    - `pub struct ThemeRegistry`
    - `ThemeRegistry::builtin() -> ThemeRegistry`. Contains "One Dark" and "One Light".
    - `load_dir(&mut self, dir: &Path) -> Vec<(PathBuf, String)>`. Returns a list of per-file errors; good files still load.
    - `get(&self, name: &str) -> Option<Arc<Theme>>`
    - `get_or_default(&self, name: &str) -> Arc<Theme>`. Falls back to "One Dark".
    - `names(&self) -> Vec<String>`
- Schema: a theme family `{ "name", "author", "themes": [ { "name", "appearance", "style": { …ui keys…, "syntax": { "<capture>": { "color", "font_style", "font_weight" } } } } ] }`.
  - `font_style == "italic"` sets italic.
  - `font_weight >= 600` sets bold.
  - Unknown keys are ignored.

- [ ] **Step 1: Write failing tests.**
  - `parse_hex_rgb_and_rgba`
  - `builtin_has_one_dark_and_light`
  - `ui_colors_present_for_one_dark`
  - `style_for_falls_back`: `"function.method.call"` resolves to the style of `function` when only `function` exists.
  - `italic_and_bold_mapping`
  - `load_dir_reports_bad_file_and_keeps_good`
  - `unknown_name_defaults_to_one_dark`
- [ ] **Step 2: Run** `cargo test -p lucerna-theme`. Expected: FAIL.
- [ ] **Step 3: Implement** with `serde`. Write `schema.rs` structs with `#[serde(default)]` and a `HashMap<String, serde_json::Value>` for the style keys.
- [ ] **Step 4: Run** `cargo test -p lucerna-theme`. Expected: PASS.
- [ ] **Step 5: Commit** with message `feat(theme): Zed v0.2.0 theme loading with One Dark/Light`.

### Task 11: Workbench shell, command registry, key dispatch

**Files:**
- Create in `crates/lucerna-workbench/src/`: `app_state.rs`, `commands.rs`, `keys.rs`, `layout/mod.rs`, `layout/activity_bar.rs`, `layout/sidebar.rs`, `layout/status_bar.rs`, `workbench.rs`
- Test: `crates/lucerna-workbench/tests/shell.rs`

**Interfaces:**
- Consumes:
  - `SettingsStore`, `Keymap`, `Context`, `Resolution` (Tasks 2–3).
  - `ThemeRegistry`, `Theme` (Task 10).
- Produces:
  - **App state:** `pub struct AppState { pub settings: SettingsStore, pub keymap: Keymap, pub themes: ThemeRegistry, pub theme: Arc<Theme> }`. It is a GPUI global.
  - **Commands:**
    - `pub struct CommandRegistry`
    - `register(&mut self, id: &'static str, title: &'static str, handler: fn(&mut Workbench, Option<&serde_json::Value>, &mut gpui::Context<Workbench>))`
    - `run(&self, id: &str, args: Option<&Value>, wb: &mut Workbench, cx)`, which returns `bool`
    - `all(&self) -> impl Iterator<Item=(&str, &str)>`
  - **Workbench view:**
    - `pub struct Workbench`, the root view.
    - `Workbench::new(root: PathBuf, cx) -> Self`
    - `context_keys(&self, window, cx) -> Context`. Sets exactly these keys:
      - `editorFocus`, `sideBarVisible`, `sideBarFocus`, `explorerViewletFocus`, `searchViewletFocus`
      - `inQuickOpen`, `findWidgetVisible`
      - `resourceExtname`: for example `".rs"`, or `""` when nothing is open
      - `editorLangId`: the `Language::id`, or `""`
      - Tasks 16–17 own `inQuickOpen` and `findWidgetVisible`; until then they are `false`.
    - `sidebar_visible(&self) -> bool`
    - `active_sidebar(&self) -> SidebarView`
    - `pub enum SidebarView { Explorer, Search }`
  - **Keys:** `keys::to_keystroke(ev: &gpui::KeyDownEvent) -> lucerna_settings::Keystroke`
- Dispatch: the root `on_key_down` collects the pending chord and calls `keymap.resolve` with the current `context_keys`.
  - `Command` runs the command and clears the pending chord.
  - `Pending` keeps the chord and shows it in the status bar.
  - `NoMatch` clears the pending chord and lets the event propagate.
  - GPUI's own action and keymap system is **not** used for user-facing bindings.
- Layout, left to right:
  - Activity bar, 48 px, with Explorer and Search icons.
  - Sidebar: default 260 px, resizable by dragging, 160–600 px.
  - Editor area (Task 13).
  - Status bar along the bottom, 22 px.
  - All colors come from `theme.ui`.
- Commands registered here:
  - `workbench.action.toggleSidebarVisibility`
  - `workbench.view.explorer`
  - `workbench.view.search`

- [ ] **Step 1: Write failing GPUI tests** using `gpui::TestAppContext`.
  - `default_layout_shows_explorer`
  - `cmd_b_toggles_sidebar`: simulate the keystroke `cmd-b`, check that `sidebar_visible() == false`, then simulate again.
  - `user_unbinding_disables_cmd_b`
  - `chord_pending_shows_in_status_bar`: `cmd-k` sets the pending state, then `escape` clears it.
  - `context_keys_reflect_focus`: `sideBarFocus` is true after focusing the sidebar.
- [ ] **Step 2: Run** `cargo test -p lucerna-workbench --test shell`. Expected: FAIL.
- [ ] **Step 3: Implement** the shell views and the dispatch.
- [ ] **Step 4: Run** `cargo test -p lucerna-workbench --test shell`. Expected: PASS. Also run `cargo run -p lucerna-workbench --example hello_window`, updated to show the `Workbench`, and check the layout by eye in both themes.
- [ ] **Step 5: Commit** with message `feat(workbench): shell layout, command registry, VS Code key dispatch`.

### Task 12: Explorer view

**Files:**
- Create: `crates/lucerna-workbench/src/explorer/mod.rs`, `explorer/model.rs`, `explorer/view.rs`
- Test: `crates/lucerna-workbench/tests/explorer_model.rs`, `tests/explorer_view.rs`

**Interfaces:**
- Consumes: `Worktree`, `WorktreeSnapshot`, `WorktreeEvent`, `Change` (Tasks 5–6). It also needs `Workbench::open_path(path, OpenMode)`, which Task 13 adds.
- Produces:
  - `pub struct ExplorerModel`. Pure, with no GPUI. Methods:
    - `new(snapshot) -> Self`
    - `set_snapshot(&mut self, snapshot)`
    - `toggle(&mut self, dir: &Path)`
    - `visible_rows(&self) -> &[Row]`
    - `reveal(&mut self, path: &Path) -> Option<usize>`. Expands the ancestors and returns the row index.
    - `mark_changed(&mut self, paths: &[Arc<Path>], now: Instant)`
    - `is_recently_changed(&self, path: &Path, now: Instant) -> bool`. True for 10 s, or until `clear_changed(path)`.
  - `pub struct Row { pub path: Arc<Path>, pub depth: u16, pub kind: EntryKind, pub expanded: bool }`
  - Uses `OpenMode` from Task 13.
- Behavior:
  - Clicking a file calls `open_path(Preview)`. Double-clicking calls `open_path(Pinned)`.
  - The expansion state survives snapshot updates for directories that still exist.
  - When the active tab changes and `explorer.autoReveal` is true, call `reveal` and scroll to the row.
  - The changed-file decoration (only if `explorer.decorations.externalChanges` is true) is the file name drawn in `theme.ui.text` bold, with a dot. Opening the file calls `clear_changed`.
  - The list is virtualized with `gpui::uniform_list`.

- [ ] **Step 1: Write failing model tests.**
  - `root_children_visible_collapsed`
  - `toggle_expands`
  - `expansion_survives_snapshot_update`
  - `reveal_expands_ancestors`
  - `changed_decoration_expires_after_10s`, driven by `Instant` arithmetic.
  - `clear_on_open`
  - `removed_dir_drops_expansion`
- [ ] **Step 2: Write failing view test** `click_opens_preview_double_click_pins`. It uses `TestAppContext` and checks the resulting tab state through Task 13's model.
- [ ] **Step 3: Run** `cargo test -p lucerna-workbench --test explorer_model --test explorer_view`. Expected: FAIL.
- [ ] **Step 4: Implement.**
- [ ] **Step 5: Run** the tests. Expected: PASS.
- [ ] **Step 6: Commit** with message `feat(workbench): virtualized explorer with reveal and change decorations`.

> Implement Task 13's `EditorGroups` model before Task 12's view test. Ordering inside the workbench: Task 13 Steps 1–4 (the model) → Task 12 → the rest of Task 13.

### Task 13: Editor groups, tabs, splits

**Files:**
- Create: `crates/lucerna-workbench/src/editor_groups/model.rs`, `editor_groups/view.rs`, `editor_groups/mod.rs`
- Test: `crates/lucerna-workbench/tests/editor_groups_model.rs`, `tests/editor_groups_view.rs`

**Interfaces:**
- Produces:
  - `pub enum OpenMode { Preview, Pinned, ToSide }`
  - `pub struct TabId(u64)`
  - `pub struct Tab { pub id: TabId, pub path: Arc<Path>, pub pinned: bool }`
  - `pub struct Group { pub tabs: Vec<Tab>, pub active: Option<TabId>, pub mru: Vec<TabId> }`
  - `pub struct EditorGroups`. Pure; group order goes left to right. Methods:
    - `open(&mut self, path: Arc<Path>, mode: OpenMode, preview_enabled: bool) -> TabId`
    - `pin(&mut self, id)`
    - `close(&mut self, id) -> Option<TabId>`. Returns the newly active tab.
    - `split_active(&mut self) -> usize`
    - `focus_group(&mut self, index: usize) -> bool`
    - `active_group(&self) -> usize`
    - `active_tab(&self) -> Option<&Tab>`
    - `cycle_mru(&mut self) -> Option<TabId>`
    - `groups(&self) -> &[Group]`
    - `restore(state: &GroupsState) -> Self`
    - `state(&self) -> GroupsState`
  - `pub struct GroupsState { pub groups: Vec<GroupState>, pub active_group: usize }`
  - `pub struct GroupState { pub tabs: Vec<(PathBuf, bool /*pinned*/)>, pub active: Option<usize> }`. These are serde types, also used by Task 19.
  - `Workbench::open_path(&mut self, path: Arc<Path>, mode: OpenMode, cx)`
- Rules:
  - **Opening a file** in a group:
    - If the file is already open there, activate that tab. In `Pinned` mode, also pin it.
    - Otherwise, in `Preview` mode with `preview_enabled`, replace the group's unpinned tab if it has one; if not, append a new unpinned tab.
    - In `Pinned` mode, or when previews are disabled, append a pinned tab.
    - `ToSide` opens the file in the next group to the right, creating that group if needed.
  - **Closing** the last tab in a non-first group removes the group. After a close, the active tab is the most recent remaining one in the MRU list.
  - **Split** duplicates the active tab into a new group to the right and focuses it.
  - **Tab bar:** unpinned tabs show an italic title. Middle-click closes a tab. Each tab has a close button.
- Commands registered:
  - `workbench.action.splitEditor`
  - `workbench.action.focusFirstEditorGroup` … `focusEighthEditorGroup`
  - `workbench.action.closeActiveEditor`
  - `workbench.action.quickOpenPreviousRecentlyUsedEditorInGroup`, which calls `cycle_mru`

- [ ] **Step 1: Write failing model tests.**
  - `preview_replaces_preview`
  - `pinned_not_replaced`
  - `open_existing_activates`
  - `preview_disabled_always_pins`
  - `to_side_creates_group`
  - `close_activates_mru`
  - `close_last_tab_removes_non_first_group`
  - `focus_group_out_of_range_false`
  - `split_duplicates_active`
  - `cycle_mru_alternates_last_two`
  - `state_roundtrip`
- [ ] **Step 2: Run** `cargo test -p lucerna-workbench --test editor_groups_model`. Expected: FAIL.
- [ ] **Step 3: Implement** `model.rs`.
- [ ] **Step 4: Run** the tests again. Expected: PASS.
- [ ] **Step 5: Write a failing view test** `cmd_backslash_splits_and_cmd_1_focuses`.
- [ ] **Step 6: Implement** `view.rs`: the tab bars and the horizontal split with draggable dividers.
- [ ] **Step 7: Run** `cargo test -p lucerna-workbench`. Expected: PASS.
- [ ] **Step 8: Commit** with message `feat(workbench): editor groups with preview tabs, splits and MRU`.

### Task 14: Code view: rendering, scrolling, selection

**Files:**
- Create: `crates/lucerna-workbench/src/code_view/mod.rs`, `code_view/layout.rs`, `code_view/element.rs`, `code_view/selection.rs`, `code_view/minimap.rs`
- Test: `crates/lucerna-workbench/tests/code_view_layout.rs`, `tests/code_view.rs`

**Interfaces:**
- Consumes:
  - `BufferSnapshot`, `Point` (Task 4).
  - `SyntaxState`, `HighlightSpan`, `indent_folds`, `matching_bracket` (Task 9).
  - `Theme::style_for` (Task 10).
- Produces:
  - `pub struct CodeView`, a GPUI view. Methods:
    - `CodeView::new(path: Arc<Path>, cx) -> Self`
    - `set_buffer(&mut self, snapshot: BufferSnapshot, cx)`
    - `scroll_top_row(&self) -> u32`
    - `set_scroll_top_row(&mut self, row: u32, cx)`
    - `selection(&self) -> Option<Range<Point>>`
    - `copy_selection(&self) -> Option<String>`
    - `toggle_word_wrap(&mut self, cx)`
    - `zoom(&mut self, delta: f32, cx)`
    - `go_to(&mut self, p: Point, cx)`. Centers the target row.
  - Pure layout in `layout.rs`:
    - `pub fn display_lines(snapshot: &BufferSnapshot, folds: &FoldState, wrap_width_chars: Option<u32>, rows: Range<u32>) -> Vec<DisplayLine>`
    - `pub struct DisplayLine { pub buffer_row: u32, pub text: String, pub truncated: bool, pub wrap_index: u16 }`
    - `pub struct FoldState`, with `toggle(&mut self, header_row: u32, available: &[Range<u32>])` and `is_hidden(&self, row: u32) -> bool`.
  - `pub const MAX_RENDERED_LINE_CHARS: usize = 10_000;`
- Behavior:
  - Gutter with line numbers; the active line uses `editor_active_line_number`.
  - Lines are virtualized. Highlights are requested for the visible rows ± 100 and cached per snapshot.
  - **Large files** (`is_large` with the settings thresholds): no highlighting, minimap or folding. The status bar shows "Large file: highlighting disabled".
  - **Long lines:** text is truncated at 10000 chars plus `…`, and `truncated = true`.
  - **Selection:** mouse drag and shift+click. Double-click selects a word. Triple-click selects a line. `editor.action.clipboardCopyAction` writes the selection to the clipboard.
  - **Word wrap** follows `editor.wordWrap` (`"off"` | `"on"`). Alt+Z toggles it.
  - **Font:** `editor.fontFamily`, `editor.fontSize`, `editor.lineHeight`. Zoom changes the size by ±1, clamped to 6–40.
  - **Minimap:** shown when `editor.minimap.enabled`. Draws one 2-px row per line using the highlight colors, plus a viewport slider. Clicking or dragging scrolls.
  - **Folding:** a gutter chevron appears on rows that start a fold.
  - **Brackets:** when the cursor sits next to a bracket, both brackets get a box.
  - `editor.renderWhitespace` (`"none"` | `"all"`) draws `·` and `→`.
- Commands registered: `editor.action.toggleWordWrap`, `editor.action.fontZoomIn`, `editor.action.fontZoomOut`, `editor.action.clipboardCopyAction`.

- [ ] **Step 1: Write failing pure tests** in `code_view_layout.rs`.
  - `long_line_truncated_at_10000`
  - `minified_5mb_single_line_layout_fast`: `display_lines` for row 0 returns within 5 ms (debug-assert timing loosened ×10) with `truncated`.
  - `wrap_splits_lines`
  - `fold_hides_rows`
  - `fold_toggle_ignores_non_header`
  - `rows_beyond_end_clamped`
- [ ] **Step 2: Run** `cargo test -p lucerna-workbench --test code_view_layout`. Expected: FAIL.
- [ ] **Step 3: Implement** `layout.rs`.
- [ ] **Step 4: Run** the layout tests. Expected: PASS.
- [ ] **Step 5: Write failing view tests** in `code_view.rs`.
  - `copy_selection_returns_text`
  - `go_to_centers_row`
  - `large_file_disables_highlight`, using a settings override of `lucerna.largeFile.maxLines = 10`.
  - `zoom_clamps`
- [ ] **Step 6: Implement** `element.rs` (a custom GPUI `Element`: shaped text runs per line, colored from `style_for`), `selection.rs` and `minimap.rs`.
- [ ] **Step 7: Run** `cargo test -p lucerna-workbench`. Expected: PASS. Then run manually: open a fixture of each bundled language and a 10 MB file, and confirm by eye that scrolling is smooth.
- [ ] **Step 8: Commit** with message `feat(workbench): virtualized code view with highlights, selection, wrap, minimap, folding`.

### Task 15: Open/reload pipeline and file states

**Files:**
- Create: `crates/lucerna-workbench/src/documents.rs`
- Modify: `crates/lucerna-workbench/src/code_view/mod.rs`, to add the error and deleted states
- Test: `crates/lucerna-workbench/tests/documents.rs`

**Interfaces:**
- Consumes:
  - `Buffer::load`, `Buffer::reload`, `Reload::adjust_row`, `LoadError` (Task 4).
  - `SyntaxState::parse` (Task 9).
  - `WorktreeEvent::Changed`, `Rescanned` (Tasks 5–6).
  - `CodeView` (Task 14).
- Produces:
  - `pub enum DocState { Loading, Ready, Error(LoadError), Deleted }`
  - `pub struct Documents`, a GPUI entity. Methods:
    - `open(&mut self, path: Arc<Path>, cx) -> Entity<CodeView>`. Deduplicated per path.
    - `state(&self, path: &Path) -> Option<&DocState>`
    - `on_worktree_event(&mut self, ev: &WorktreeEvent, cx)`
    - `retry(&mut self, path, cx)`
    - `open_as_text(&mut self, path, cx)`
- Pipeline:
  1. Load on a background thread, then call `set_buffer`.
  2. `SyntaxState::parse(snapshot, Some(SYNC_PARSE_BUDGET))`. On `TimedOut`, call `parse(…, None)` on a background thread and repaint when it finishes.
  3. Only the active tab is loaded at startup. Other tabs load when activated.
- External changes, for paths that are open:
  - **`Modified`:** reload on a background thread. Map the old scroll top through `adjust_row` with the new line count, parse again, and repaint. Selections are cleared if their rows changed.
  - **`Removed`:** `Deleted`. The tab title gets a strikethrough, and the last content stays visible.
  - **`Added`** of a path in the `Deleted` state: reload it and return to `Ready`.
  - **`Rescanned`:** check the mtime of every open file and reload the ones that changed. Open files that no longer exist become `Deleted`.
- Error UI:

  | Error | Shown in the tab |
  |---|---|
  | `NotFound` / `PermissionDenied` / `Io` | Message plus a "Retry" button |
  | `Binary` | "Binary file" plus an "Open as text" button |
  | `TooLarge` | "File is larger than 512 MB" |

- Status bar for the active document: `Ln {row+1}, Col {col+1}`, the encoding name, `LF`/`CRLF`, and the language name.

- [ ] **Step 1: Write failing tests** with `TestAppContext` and a temp dir, sending synthetic `WorktreeEvent`s.
  - `opens_and_highlights`
  - `modify_reload_keeps_scroll_when_change_below`
  - `insert_above_shifts_scroll`
  - `truncate_to_empty_clamps_scroll_no_panic`
  - `removed_marks_deleted_and_keeps_content`
  - `readded_returns_ready`
  - `binary_shows_open_as_text_then_opens`
  - `permission_denied_retry`
  - `rescan_reloads_changed_open_files`
- [ ] **Step 2: Run** `cargo test -p lucerna-workbench --test documents`. Expected: FAIL.
- [ ] **Step 3: Implement** `documents.rs` and the state rendering.
- [ ] **Step 4: Run** the tests. Expected: PASS.
- [ ] **Step 5: Commit** with message `feat(workbench): document pipeline with live reload and error states`.

### Task 16: Quick Open and command palette

**Files:**
- Create: `crates/lucerna-workbench/src/quick_open/mod.rs`, `quick_open/query.rs`, `quick_open/view.rs`
- Test: `crates/lucerna-workbench/tests/quick_open_query.rs`, `tests/quick_open.rs`

**Interfaces:**
- Consumes:
  - `match_files`, `QUICK_OPEN_LIMIT` (Task 7).
  - `CommandRegistry::all`, `Keymap::label_for` (Tasks 3, 11).
  - `EditorGroups` MRU and `open_path` (Task 13).
  - `CodeView::go_to` (Task 14).
- Produces:
  - `pub enum QuickOpenQuery { Files(String), Commands(String), GotoLine { line: Option<u32>, column: Option<u32> } }`
  - `pub fn parse_query(input: &str) -> QuickOpenQuery`. A leading `>` gives Commands; a leading `:` gives GotoLine. `":12:5"` gives line 12 and column 5. Non-numeric parts become `None`.
  - `pub struct QuickOpen`, a modal GPUI view, opened with an initial prefix.
- Behavior:
  - **Files mode:**
    - Runs `match_files` on a background thread, cancelling the previous query on each keystroke.
    - `recent` is the global MRU of opened paths, newest first, up to 50.
    - Rows show the file name with matched characters bold, then the directory dimmed.
    - Enter calls `open_path(Preview)`. Cmd+Enter calls `open_path(ToSide)`.
  - **Commands mode:**
    - Filters titles and IDs with `nucleo` over `CommandRegistry::all`.
    - Shows the keybinding label from `label_for` on the right.
    - Enter runs the command.
  - **GotoLine mode:**
    - The row reads "Go to line N" ("Go to line N, column C" when a column is given), or "Type a line number…" when none is typed.
    - Enter calls `go_to`, with the 1-based input converted to 0-based.
  - Up and Down move the selection. Escape closes. Clicking outside closes.
  - The `inQuickOpen` context key is true while the widget is open.
- Commands registered:
  - `workbench.action.quickOpen` (empty prefix)
  - `workbench.action.showCommands` (prefix `>`)
  - `workbench.action.gotoLine` (prefix `:`)
  - `workbench.action.closeQuickOpen`

- [ ] **Step 1: Write failing tests** in `quick_open_query.rs`.
  - `plain_is_files`
  - `gt_is_commands_trimmed`
  - `colon_line`
  - `colon_line_col`
  - `colon_garbage_none`
- [ ] **Step 2: Write failing view tests** in `quick_open.rs`.
  - `cmd_p_opens_and_enter_opens_preview`
  - `cmd_enter_opens_to_side`
  - `commands_mode_runs_toggle_sidebar`
  - `goto_line_moves_view`
  - `escape_closes_and_context_key_resets`
- [ ] **Step 3: Run** `cargo test -p lucerna-workbench --test quick_open_query --test quick_open`. Expected: FAIL.
- [ ] **Step 4: Implement.**
- [ ] **Step 5: Run** the tests. Expected: PASS.
- [ ] **Step 6: Commit** with message `feat(workbench): Quick Open with files, commands and go-to-line`.

### Task 17: Find in file and project search panel

**Files:**
- Create: `crates/lucerna-workbench/src/find_widget.rs`, `crates/lucerna-workbench/src/search_panel.rs`
- Test: `crates/lucerna-workbench/tests/find_widget.rs`, `tests/search_panel.rs`

**Interfaces:**
- Consumes:
  - `search`, `SearchQuery`, `SearchEvent`, `MAX_RESULTS` (Task 8).
  - `BufferSnapshot` (Task 4).
  - `CodeView::go_to` (Task 14).
  - The `search.exclude` setting (Task 2).
- Produces:
  - `pub fn find_in_snapshot(snapshot: &BufferSnapshot, pattern: &str, is_regex: bool, case_sensitive: bool, whole_word: bool) -> Result<Vec<Range<usize>>, String>`. Pure; builds the regex the same way as Task 8.
  - `FindWidget`, which hangs over the top-right of the `CodeView`:
    - Input with toggles Aa / ab / .*, and a counter "N of M".
    - Enter goes to the next match, Shift+Enter to the previous.
    - Matches are drawn with `search_match_background`.
    - `findWidgetVisible` is true while it is shown.
  - `SearchPanel`, the sidebar view `Search`:
    - A query input with the same three toggles, plus "files to include" and "files to exclude" inputs.
    - Results grouped by file, collapsible.
    - Shows "N results in M files", plus "(truncated at 20,000)" when truncated.
- Behavior:
  - Searches are debounced by 150 ms. Changing the query cancels the running search through its `AtomicBool`.
  - Clicking a result calls `open_path(Preview)` and then `go_to`, and highlights the match.
  - An invalid regex shows the error under the input.
- Commands registered: `actions.find`, `closeFindWidget`, `workbench.action.findInFiles` (focuses the panel input and pre-fills the selection when there is one).

- [ ] **Step 1: Write failing tests.**
  - `find_in_snapshot_literal_regex_case_word`
  - `find_invalid_regex_err`
  - `find_widget_next_prev_wraps`
  - `search_panel_streams_and_groups`
  - `search_change_cancels_previous`, which checks that no results from the old query arrive after the change.
  - `search_truncation_message`
  - `click_result_opens_and_positions`
- [ ] **Step 2: Run** `cargo test -p lucerna-workbench --test find_widget --test search_panel`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the tests. Expected: PASS.
- [ ] **Step 5: Commit** with message `feat(workbench): find widget and streaming project search panel`.

### Task 18: Settings files, hot reload, theme picker

**Files:**
- Create: `crates/lucerna-workbench/src/config_files.rs`, `crates/lucerna-workbench/src/theme_picker.rs`
- Test: `crates/lucerna-workbench/tests/config_files.rs`

**Interfaces:**
- Consumes:
  - `SettingsStore::set_layer`, `Keymap::load`, `SettingsError` (Tasks 2–3).
  - `ThemeRegistry::load_dir`, `get_or_default` (Task 10).
  - `Documents::open` (Task 15).
  - `QuickOpen`'s list rendering (Task 16).
- Produces:
  - `pub struct ConfigPaths { pub user_dir: PathBuf, pub workspace_settings: PathBuf }`
  - `ConfigPaths::for_root(root: &Path) -> ConfigPaths`. `user_dir` is `~/.config/lucerna`; workspace settings are at `<root>/.lucerna/settings.json`.
  - `ConfigPaths::with_user_dir(user_dir, root)`, for tests.
  - `pub fn watch_config(paths: ConfigPaths, cx)`. Uses its own `notify` watcher on the user directory and the workspace `.lucerna` directory, debounced by 50 ms.
- Behavior:
  - **Reload:** on change, re-read the file.
    - Settings go through `set_layer`.
    - Keybindings go through `Keymap::load`.
    - User themes go through `load_dir`.
    - Then apply the result: re-render, re-theme, and rebuild `ScanOptions` if `files.exclude` changed.
  - **Errors:** on `SettingsError`, the status bar shows a red item "settings.json: line N: msg". Clicking it opens the file and calls `go_to`. The error is cleared on the next successful parse.
  - **Missing files:** if `settings.json` or `keybindings.json` does not exist, the open command creates it containing `// Lucerna user settings\n{\n}\n` (or `[\n]` for keybindings) before opening.
  - **Theme picker:** reuses the QuickOpen list over `ThemeRegistry::names()`. Moving the selection previews the theme live, Escape reverts it, and Enter writes `"workbench.colorTheme"` into the user `settings.json`. Writing replaces the existing key's value, or appends the key before the final `}`, and preserves comments elsewhere.
- Commands registered:

  | Command | Title |
  |---|---|
  | `workbench.action.openSettingsJson` | — |
  | `workbench.action.openGlobalKeybindingsFile` | — |
  | `workbench.action.openWorkspaceSettingsFile` | — |
  | `workbench.action.selectTheme` | — |
  | `lucerna.openLogsFolder` | "Lucerna: Open Logs Folder" (runs `open ~/Library/Logs/Lucerna`) |

- [ ] **Step 1: Write failing tests**, using `ConfigPaths::with_user_dir` pointed at a temp dir.
  - `user_settings_change_applies`
  - `invalid_settings_shows_error_keeps_previous`
  - `fixing_settings_clears_error`
  - `keybindings_change_applies`
  - `files_exclude_change_rescans`
  - `open_settings_creates_missing_file`
  - `theme_picker_writes_setting_preserving_comments`
  - `theme_picker_escape_reverts`
- [ ] **Step 2: Run** `cargo test -p lucerna-workbench --test config_files`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the tests. Expected: PASS.
- [ ] **Step 5: Commit** with message `feat(workbench): hot-reloaded settings/keybindings and theme picker`.

### Task 19: Workspace state persistence

**Files:**
- Create: `crates/lucerna-workbench/src/workspace_state.rs`
- Test: `crates/lucerna-workbench/tests/workspace_state.rs`

**Interfaces:**
- Consumes: `GroupsState` (Task 13), `CodeView::scroll_top_row` and `set_scroll_top_row` (Task 14).
- Produces:
  - `pub struct WorkspaceState { pub version: u32 /* = 1 */, pub sidebar_visible: bool, pub sidebar_width: f32, pub sidebar_view: String, pub groups: GroupsState, pub scroll: HashMap<PathBuf, u32> }`
  - `pub fn state_path(data_dir: &Path, root: &Path) -> PathBuf`. Returns `<data_dir>/workspaces/<hex sha256 of canonical root>.json`. In the app, `data_dir` is `~/Library/Application Support/Lucerna`.
  - `pub fn load_state(path: &Path) -> Option<WorkspaceState>`
  - `pub fn save_state(path: &Path, s: &WorkspaceState) -> io::Result<()>`
- Rules:
  - **Save:** write `*.tmp`, then rename. Save on window close, and at most once every 2 s after a layout change.
  - **Load:**
    - A missing file gives `None`.
    - A parse error or a version mismatch renames the file to `<name>.json.bak`, overwriting any previous `.bak`, and returns `None`.
    - Tabs whose file no longer exists are dropped.

- [ ] **Step 1: Write failing tests.**
  - `roundtrip`
  - `same_root_same_path_different_root_different_path`
  - `corrupt_file_renamed_bak_returns_none`
  - `missing_files_dropped_on_restore`
  - `restore_applies_scroll_and_sidebar`, a `TestAppContext` test.
- [ ] **Step 2: Run** `cargo test -p lucerna-workbench --test workspace_state`. Expected: FAIL.
- [ ] **Step 3: Implement** with `sha2` and `serde_json`.
- [ ] **Step 4: Run** the tests. Expected: PASS.
- [ ] **Step 5: Commit** with message `feat(workbench): persist and restore workspace layout`.

### Task 20: `luc` app wiring, logging, panic isolation

**Files:**
- Modify: `crates/lucerna/src/main.rs`
- Create: `crates/lucerna/src/logging.rs`, `crates/lucerna-workbench/src/tasks.rs`
- Test: `crates/lucerna/tests/cli.rs`, `crates/lucerna-workbench/tests/tasks.rs`

**Interfaces:**
- Consumes: everything above.
- Produces:
  - `pub fn resolve_target(path: Option<PathBuf>) -> Result<(PathBuf /*root*/, Option<PathBuf> /*file to open*/), String>`
  - `pub fn spawn_guarded<T: Send + 'static>(name: &'static str, f: impl FnOnce() -> T + Send + 'static) -> std::thread::JoinHandle<Option<T>>`. Lives in `lucerna-workbench::tasks`. It wraps `f` in `catch_unwind`, logs the panic with `tracing::error!(task = name, …)`, and returns `None`.
  - Logging: `logging::init() -> tracing_appender::non_blocking::WorkerGuard`. Writes daily-rolling files to `~/Library/Logs/Lucerna/`, keeping 7 files (`max_log_files(7)`). The log level comes from `LUCERNA_LOG` (default `info`).
  - Flag `--trace-startup`: logs and prints `startup_ms=<n>` to stderr when the first frame is painted.
- `resolve_target` rules:
  - `None`: the current directory.
  - A directory: that directory.
  - A file:
    1. If the file is inside a git repo, the root is the nearest ancestor containing `.git`.
    2. Otherwise, the root is the file's parent directory.
    3. In both cases, the file is opened as a pinned tab.
  - A path that does not exist: `Err("path not found: …")`, exit code 1.
- All background work in Tasks 5–19 (loads, parses, searches, matching) goes through `spawn_guarded`. Replace any direct `std::thread::spawn` calls.

- [ ] **Step 1: Write failing tests.**
  - `resolve_none_is_cwd`
  - `resolve_dir`
  - `resolve_file_in_git_repo_uses_repo_root`
  - `resolve_file_outside_git_uses_parent`
  - `resolve_missing_errors`
  - `missing_path_exits_1`, a CLI test.
  - `spawn_guarded_catches_panic_returns_none`
  - `spawn_guarded_returns_value`
- [ ] **Step 2: Run** `cargo test -p lucerna -p lucerna-workbench --test cli --test tasks`. Expected: FAIL.
- [ ] **Step 3: Implement**, and wire `main`:
  1. Initialize logging.
  2. Resolve the target.
  3. Load settings, keymap and themes.
  4. Start the `Worktree`.
  5. Restore the workspace state.
  6. Open the `Workbench` window.
  7. Open the file argument, if any.
- [ ] **Step 4: Run** `cargo test --workspace`. Expected: PASS.
- [ ] **Step 5: Manual smoke test:** `cargo run --release -p lucerna -- .` on this repo.
  1. Browse the tree.
  2. Use Cmd+P, Cmd+Shift+F, split, and the theme picker.
  3. Edit a file from a terminal with `echo x >> README.md`. It updates within 100 ms.
  4. Quit and relaunch. The layout is restored.
- [ ] **Step 6: Commit** with message `feat: wire luc app with logging, target resolution and guarded tasks`.

### Task 21: Benchmarks and performance gate

**Files:**
- Create:
  - `crates/lucerna-project/benches/scan.rs`
  - `crates/lucerna-project/benches/fuzzy.rs`
  - `crates/lucerna-syntax/benches/highlight.rs`
  - `crates/lucerna-text/benches/reload.rs`
  - `scripts/gen-bench-repo.sh`
  - `docs/perf-checklist.md`
- Modify: `.github/workflows/ci.yml`

**Interfaces:**
- Consumes: `Worktree::start`, `match_files`, `SyntaxState`, `Buffer::reload`.
- Produces:
  - Criterion bench groups `scan_80k`, `fuzzy_80k`, `highlight_10mb_viewport` and `reload_10mb`.
  - CI job `bench`:
    - On pushes to `main`, run `cargo bench -- --save-baseline main` and cache `target/criterion`.
    - On PRs, run `cargo bench -- --baseline main`.
    - Fail if any group regresses by more than 15%, using `critcmp --threshold 15`, or a script over the criterion JSON if `critcmp` lacks a fail mode.
- `gen-bench-repo.sh <dir>` creates 80,000 files: 4,000 dirs × 20 files with Rust- and TS-like content, plus a 10 MB `big.rs`. It is deterministic, with a fixed seed.
- `docs/perf-checklist.md` lists the spec's six targets, each with how to measure it:

  | Target | How to measure |
  |---|---|
  | First frame | `luc --trace-startup` |
  | Tree usable, scan done | Log timestamps on the first `ScanProgress` and on `ScanComplete` |
  | Quick Open | The `fuzzy_80k` bench, per call |
  | Scrolling 120 fps | Instruments' Core Animation FPS |
  | Idle RAM | Activity Monitor after 60 s idle with 10 tabs |
  | Disk → screen | Log the timestamp at the watcher event and at repaint |

- [ ] **Step 1: Write** the benches and the script.
- [ ] **Step 2: Run** `scripts/gen-bench-repo.sh /tmp/lucerna-bench && LUCERNA_BENCH_REPO=/tmp/lucerna-bench cargo bench`. Expected: all groups run.
  - Record the numbers in `docs/perf-checklist.md`.
  - `fuzzy_80k` must be < 16 ms per call.
  - `scan_80k` must be < 5 s.
- [ ] **Step 3: Walk through the manual checklist** on a release build and record the results.
  - Any target that misses gets a follow-up task added to this plan **before** Phase 1 is declared done.
- [ ] **Step 4: Commit** with message `perf: criterion benches, bench repo generator, CI regression gate, perf checklist`.
