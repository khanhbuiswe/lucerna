# Explorer Context Menu — Part 2 (New File/Folder, Rename, Delete) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The Explorer menu and keys can create files and folders and rename them by typing the name in the tree, and can move entries to the Trash (or delete them permanently) after asking, with open tabs following every change.

**Architecture:** Pure file operations and name checks live in `lucerna-project::fs_ops` (no GPUI). `ExplorerModel` holds an optional `EditState`; `ExplorerView` draws it as an extra row with a `TextInput`. Operations run on the background executor; on success the workbench re-keys `Documents` and editor-group tabs (rename) or closes clean tabs (delete) **before** the watcher event arrives, and records the paths in a short-lived "own changes" set so the Explorer does not mark them as external.

**Tech Stack:** Rust, GPUI, crate `trash` (MIT).

**Spec:** `docs/superpowers/specs/2026-11-13-explorer-context-menu-design.md` (sections 4, 5, 6, 7, 8). Builds on Part 1: `docs/superpowers/plans/2026-11-13-explorer-context-menu-p1.md` (must be merged first: `ContextMenu`, `Selection`, `build_menu`, `explorer/commands.rs`, `command_target`, context keys).

## Global Constraints

- All Part 1 constraints apply (gpui only in workbench, VS Code IDs and macOS keys, explicit-path commits, prune before tests, no Zed code).
- Never overwrite: create and rename fail with `AlreadyExists` when the target exists, except a rename that only changes letter case of the same entry.
- The UI thread never waits on the file system: every `fs_ops` call runs in `cx.background_spawn`.
- Exact copy (VS Code):
  - Exists: `A file or folder {name} already exists at this location. Please choose a different name.` (VS Code bolds `{name}`; Lucerna shows plain text)
  - Empty: `A file or folder name must be provided.`
  - Leading slash: `A file or folder name cannot start with a slash.`
  - Invalid: `The name {name} is not valid as a file or folder name. Please choose a different name.`
  - Trash prompt: message `Are you sure you want to delete '{name}'?`, detail `You can restore this file from the Trash.` (folder: `You can restore this folder and its contents from the Trash.`), buttons `Move to Trash`, `Cancel`.
  - Trash failed: `Unable to move '{name}' to the Trash. Do you want to permanently delete instead?`, buttons `Delete Permanently`, `Cancel`.
  - Permanent: `Are you sure you want to permanently delete '{name}'?`, detail `This action is irreversible!`, buttons `Delete`, `Cancel`.
- Add `trash = "5"` to `[workspace.dependencies]` and `lucerna-project`; `cargo deny check` must pass.

## Review Focus

- **Rename a file with unsaved edits:** edits, cursor, pin state and undo survive; the document is now keyed by the new path; saving writes the new path, and nothing marks it deleted (Task 4 test `rename_keeps_unsaved_edits`).
- **Rename a folder holding open files:** every tab under it follows (Task 4 test `rename_folder_moves_open_tabs`).
- **Case-only rename on macOS (`readme.md` → `README.md`):** succeeds, no "already exists" (Task 1 test `case_only_rename_is_allowed`).
- **Delete a folder with one dirty and one clean open file:** clean tab closes, dirty tab stays as deleted (Task 5 test `delete_closes_clean_tabs_keeps_dirty`).
- **Focus leaves the name input with an invalid name** (e.g. clicking the editor while it says "already exists"): the edit is cancelled, nothing is created (Task 3 test `blur_with_error_cancels`).

---

### Task 1: `fs_ops` in `lucerna-project`

**Files:**
- Create: `crates/lucerna-project/src/fs_ops.rs`; Modify: `crates/lucerna-project/src/lib.rs` (`pub mod fs_ops;`), `crates/lucerna-project/Cargo.toml`, root `Cargo.toml`
- Test: `crates/lucerna-project/tests/fs_ops.rs`

**Interfaces:**
- Produces (all paths absolute except `name`):
  - `pub enum NameError { Empty, LeadingSlash, Invalid(String), Exists(String) }` with `impl Display` giving the exact copy above (`{name}` = the offending component).
  - `pub fn validate_new_name(parent: &Path, name: &str, original: Option<&Path>, exists: impl Fn(&Path) -> bool) -> Result<PathBuf, NameError>` — pure (the caller supplies `exists`: the Explorer passes a snapshot lookup, tests pass `|p| p.exists()`). Trims trailing whitespace like VS Code; rejects empty, leading `/` or `\`, any `.`/`..` component, components invalid on this platform (`\0` everywhere; on Windows also `<>:"|?*` and reserved names). Splits `name` on `/` (multi-level). `Exists` when `exists(target)` and the target is not `original` compared case-insensitively. Returns the target path.
  - `pub fn create_file(path: &Path) -> io::Result<()>` — `create_dir_all(parent)` then `OpenOptions::new().write(true).create_new(true)`.
  - `pub fn create_dir(path: &Path) -> io::Result<()>` — `create_dir_all`, but `AlreadyExists` if `path` exists.
  - `pub fn rename(from: &Path, to: &Path) -> io::Result<()>` — `AlreadyExists` if `to` exists and is not `from` in a different case; `create_dir_all(to.parent())`; `std::fs::rename`.
  - `pub fn trash(path: &Path) -> Result<(), TrashError>` (`trash::delete`); `pub struct TrashError(pub String)`.
  - `pub fn delete_permanently(path: &Path) -> io::Result<()>` (`remove_dir_all` for dirs, else `remove_file`).
  - `pub fn stem_range(name: &str, is_dir: bool) -> Range<usize>` — byte range selected on rename: whole name for dirs and dotfiles without another dot; else `0..last_dot`.

- [ ] **Step 1: Write failing tests** in `tests/fs_ops.rs` (tempdir):
  - `validate_rejects_bad_names`: `""`, `"   "` → `Empty`; `"/a"` → `LeadingSlash`; `"a/../b"` → `Invalid`; existing `a.rs` → `Exists("a.rs")`; `"x/y/z.rs"` → `Ok(parent/x/y/z.rs)`.
  - `create_file_makes_parents_and_never_overwrites`: create `x/y/z.rs` → exists; again → `ErrorKind::AlreadyExists`, content untouched.
  - `rename_refuses_existing_target`.
  - `case_only_rename_is_allowed`: `readme.md` → `README.md` works; `validate_new_name(parent, "README.md", Some(readme), |p| p.exists())` is `Ok`.
  - `trash_removes_entry` (skip with a message when `trash::delete` errs in the environment) and `delete_permanently_removes_dir_tree`.
  - `stem_range_cases`: `"foo.rs"` → `0..3`; `"foo.tar.gz"` → `0..7`; `".gitignore"` → `0..10`; dir `"a.b"` → `0..3`.
- [ ] **Step 2:** `cargo test -p lucerna-project --test fs_ops` → FAIL.
- [ ] **Step 3:** Implement; add `trash`.
- [ ] **Step 4:** Same → PASS; `cargo deny check` → ok.
- [ ] **Step 5: Commit** — `feat(project): file operations for the Explorer`.

### Task 2: Own changes are not external

**Files:**
- Modify: `crates/lucerna-workbench/src/workbench.rs` (`own_changes: HashMap<Arc<Path>, Instant>`, `pub(crate) fn expect_own_changes(&mut self, rel_paths: impl IntoIterator<Item = Arc<Path>>)`, filter in `on_worktree_event`)
- Test: `crates/lucerna-workbench/tests/explorer_decorations.rs`

**Interfaces:**
- Produces: `Workbench::expect_own_changes` (root-relative paths, the same keys `Change::path` uses). In `on_worktree_event`, a `Change` whose path is in `own_changes` and younger than `OWN_CHANGE_TTL = Duration::from_secs(5)` is not decorated; entries older than the TTL are dropped on each event. An expected directory also covers paths under it (folder create with multi-level names).

- [ ] **Step 1: Write failing test** `own_changes_are_not_decorated`: `expect_own_changes([p("new.rs")])`, deliver `Changed([Added new.rs, Added other.rs])` → `new.rs` not recently changed, `other.rs` is.
- [ ] **Step 2:** Run → FAIL. **Step 3:** Implement. **Step 4:** Run → PASS.
- [ ] **Step 5: Commit** — `feat(explorer): don't mark Lucerna's own file operations as external`.

### Task 3: Inline name editing (New File, New Folder)

**Files:**
- Modify: `crates/lucerna-workbench/src/text_input.rs` (`pub fn select_range(&mut self, range: Range<usize>, window, cx)`)
- Modify: `crates/lucerna-workbench/src/explorer/model.rs` (`EditState`, `visible_rows` with the edit row)
- Modify: `crates/lucerna-workbench/src/explorer/view.rs` (edit row rendering, confirm/cancel, error popover)
- Modify: `crates/lucerna-workbench/src/explorer/menu.rs`, `explorer/commands.rs`
- Test: `crates/lucerna-workbench/tests/text_input.rs`, `tests/explorer_model.rs`, new `tests/explorer_edit.rs`

**Interfaces:**
- Consumes: Task 1 `validate_new_name`, `create_file`, `create_dir`; Task 2 `expect_own_changes`; Part 1 `Selection`, `build_menu`, `command_target`.
- Produces:
  - `pub enum EditKind { NewFile, NewFolder, Rename }`; `pub struct EditState { pub kind: EditKind, pub parent: Arc<Path> /*root-relative, "" = root*/, pub original: Option<Arc<Path>> }`.
  - `ExplorerModel::start_edit(&mut self, e: EditState)` (expands `parent`), `edit(&self) -> Option<&EditState>`, `end_edit(&mut self)`.
  - `Row` gains `pub edit: bool`. For New*, `visible_rows()` inserts one row with `edit: true`, `kind` = File/Dir, `depth` = parent's depth + 1, as the first child of `parent`. For Rename, the existing row gets `edit: true`.
  - `ExplorerView::start_new(kind: EditKind, window, cx)`: dir = selection if dir/root, else the selected file's parent; makes `name_input: Entity<TextInput>`, focuses it.
  - Debug selectors: `explorer-edit-input`, `explorer-edit-error`.
- Commands: `explorer.newFile` "File: New File…", `explorer.newFolder` "File: New Folder…" (no default keys, as VS Code). Menu: both first, only for `Dir`/`Root`, not in single-file mode.
- Behaviour: on every change → `validate_new_name` with `exists` = a lookup in the worktree snapshot (no disk I/O on the UI thread); the real create re-checks (`AlreadyExists` → the same `Exists` error under the input, edit stays open). `enter` with no error → spawn create; on Ok: `expect_own_changes`, `end_edit`, select the new path, then file → `open_path(new, Pinned)`; folder → expand. `escape` → `end_edit`. Blur (`on_blur` of the input focus) → confirm when valid, else cancel. While editing, `filesExplorerFocus` stays true, so add `&& !textInputFocus` to every Explorer binding Part 1 added in `default-keybindings.json`; additionally `ExplorerView` ignores its own commands while `edit().is_some()` (covers the one-frame-stale key).
- Look: the input fills the name slot, height `TREE_ROW_HEIGHT`, 1 px `border_focused`, no radius, `editor_background`. Error: a box just under the row, full row width, 1 px `error` border, `elevated_surface_background`, `UI_FONT_SIZE_SMALL`, wrapping, drawn with `deferred(anchored())` so it overlays rows below.

- [ ] **Step 1: Write failing tests:**
  - `tests/text_input.rs` `select_range_selects_bytes`: text `"foo.rs"`, `select_range(0..3)` → `selected_range() == 0..3`.
  - `tests/explorer_model.rs` `new_file_edit_row_is_first_child`: expand-less `src` with `start_edit(NewFile, parent "src")` → `visible_rows` paths `["src", <edit>, "src/main.rs", "a.rs", "b.rs"]`, edit row depth 1, `src` expanded.
  - `tests/explorer_edit.rs` (helpers from `explorer_view.rs`; `type_text(cx, s)` = `simulate_input`):
    - `new_file_creates_and_opens`: right-click `src` → click `context-menu-item-explorer.newFile` → type `util.rs` → `enter`; wait until `<root>/src/util.rs` exists; deliver `Changed([Added src/util.rs])` → tab `src/util.rs` pinned, selection `src/util.rs`, not recently changed.
    - `new_folder_multi_level`: on root, `explorer.newFolder`, type `a/b` → dirs exist, `a` and `a/b` expanded, selection `a/b`.
    - `existing_name_shows_error_and_enter_does_nothing`: type `a.rs` → `explorer-edit-error` drawn with `A file or folder a.rs already exists`; `enter` → still editing, nothing created.
    - `escape_cancels`: no file, no edit row.
    - `blur_with_error_cancels`: type `a.rs`, click the editor area → edit gone, nothing created.
    - `blur_with_valid_name_creates`: type `c.rs`, click the editor area → `c.rs` exists.
    - `create_error_shows_status`: parent made read-only (`set_permissions` 0o555, `#[cfg(unix)]`) → status message mentions the error, edit row gone.
- [ ] **Step 2:** Run → FAIL. **Step 3:** Implement. **Step 4:** Run → PASS.
- [ ] **Step 5: Commit** — `feat(explorer): New File and New Folder with inline names`.

### Task 4: Rename

**Files:**
- Modify: `crates/lucerna-workbench/src/documents.rs` (`pub fn rename(&mut self, from: &Path, to: &Path, cx)`)
- Modify: `crates/lucerna-workbench/src/code_view/mod.rs` (`pub(crate) fn set_path(&mut self, path: Arc<Path>, cx)`)
- Modify: `crates/lucerna-workbench/src/editor_groups/model.rs` (`pub fn retarget(&mut self, from: &Path, to: &Path) -> bool`)
- Modify: `crates/lucerna-workbench/src/workbench.rs` (`pub(crate) fn paths_renamed(&mut self, from: &Path, to: &Path, cx)`: documents, groups, recent files, nav history entries under `from`)
- Modify: `explorer/view.rs`, `explorer/menu.rs`, `explorer/commands.rs`, `default-keybindings.json`
- Test: `tests/editor_groups_model.rs`, `tests/documents.rs`, `tests/explorer_edit.rs`

**Interfaces:**
- `EditorGroups::retarget(from, to)`: every `TabItem::File(p)` with `p == from` or `p.starts_with(from)` becomes `to.join(p.strip_prefix(from))`; tab ids, order, pin and preview state unchanged. Returns whether anything changed.
- `Documents::rename(from, to)`: re-key every doc whose key is `from` or under it; for each: set the new key, call `set_path` on its live views, re-resolve the language (`language_for_file`) and emit `DocumentEvent::Closed { old }` then `DocumentEvent::Opened { path: new, language_id, snapshot }` so language servers move it; dirty state, undo stack, stamp and `base` are kept.
- Command `renameFile` "File: Rename…": not for `Root` or single-file. Starts `EditState { kind: Rename, parent: <entry's parent>, original: Some(entry) }`; input text = file name, `select_range(stem_range(name, is_dir))`. Confirm: unchanged name → just end; else validate (with `original`), spawn `fs_ops::rename`; on Ok → `expect_own_changes([old, new])`, `paths_renamed(abs_old, abs_new)`, select new path.
- Key: `enter` → `renameFile`, `when: filesExplorerFocus && !explorerResourceIsRoot && !textInputFocus`.
- Menu: `Rename…` in the last group, not for `Root`.

- [ ] **Step 1: Write failing tests:**
  - `editor_groups_model.rs` `retarget_moves_file_and_children`: tabs `/r/a.rs`, `/r/src/x.rs`, `/r/srcx.rs`; `retarget("/r/src", "/r/lib")` → `/r/a.rs`, `/r/lib/x.rs`, `/r/srcx.rs` (prefix is per component); ids unchanged.
  - `documents.rs` `rename_rekeys_dirty_document`: open `a.rs`, edit, `rename(a.rs, b.rs)` → `is_dirty(b.rs)`, `!is_dirty(a.rs)`, `views_of(b.rs)` has the view whose `path()` is `b.rs`; events contain `Closed{a.rs}` then `Opened{b.rs}`.
  - `explorer_edit.rs`:
    - `rename_selects_stem`: select `a.rs`, `enter` → input text `a.rs`, selected range `0..1`.
    - `rename_keeps_unsaved_edits`: open `a.rs` pinned, type in the editor, rename to `c.rs` → the tab shows `c.rs`, still dirty, cursor unchanged; deliver `Changed([Removed a.rs, Added c.rs])` → doc state not `Deleted`; `cmd-s` writes `<root>/c.rs`.
    - `rename_folder_moves_open_tabs`: open `src/main.rs`, rename `src` → `lib` → tab path `lib/main.rs`.
    - `rename_to_existing_shows_error`: `a.rs` → `b.rs` → error, nothing moved.
    - `case_only_rename`: `a.rs` → `A.rs` works (on macOS's case-insensitive temp dir too).
    - `enter_on_root_does_nothing`.
- [ ] **Step 2:** Run → FAIL. **Step 3:** Implement. **Step 4:** Run → PASS (also the full `documents` and `lsp_sync` test files, since `Closed`/`Opened` drive servers).
- [ ] **Step 5: Commit** — `feat(explorer): rename in place, tabs follow`.

### Task 5: Delete

**Files:**
- Modify: `explorer/commands.rs`, `explorer/menu.rs`, `workbench.rs` (`pub(crate) fn paths_deleted(&mut self, abs: &Path, cx)`), `default-settings.json`, `default-keybindings.json`
- Test: `tests/explorer_edit.rs`

**Interfaces:**
- Consumes: Task 1 `trash`, `delete_permanently`; Task 2.
- `moveFileToTrash` "File: Delete": not for `Root` or single-file. If `explorer.confirmDelete` (default `true`) → `window.prompt(PromptLevel::Warning, msg, Some(detail), &["Move to Trash", "Cancel"], cx)`; on `Move to Trash` (or confirm off) → background `fs_ops::trash`; on `Err` → second prompt (`Delete Permanently` / `Cancel`) → `delete_permanently`. On success → `expect_own_changes([rel])`, `paths_deleted(abs)`, select the parent.
- `deleteFile` "File: Delete Permanently": always prompts (permanent copy above), then `delete_permanently`.
- `paths_deleted(abs)`: close every tab whose path is `abs` or under it **and** is not dirty (`documents.is_dirty`); dirty ones stay (the watcher's `Removed` sets `DocState::Deleted` as today).
- Settings: `"explorer.confirmDelete": true`. Keys: `cmd+backspace` → `moveFileToTrash`, `alt+cmd+backspace` → `deleteFile`, both `when: filesExplorerFocus && !explorerResourceIsRoot && !textInputFocus`.
- Menu: `Delete` after `Rename…`, not for `Root`.

- Extends Part 1's `SystemShell` with `fn trash(&self, path: &Path) -> Result<(), String>` (`NativeShell` → `fs_ops::trash`), so tests choose success or failure without a real Trash.

- [ ] **Step 1: Write failing tests** (`cx.simulate_prompt_answer`, `cx.has_pending_prompt`, a recording `SystemShell`):
  - `delete_asks_then_trashes`: select `a.rs`, `cmd-backspace` → prompt pending; answer `Move to Trash` → recorded `trash(<root>/a.rs)`.
  - `cancel_keeps_file`: answer `Cancel` → no trash call.
  - `confirm_off_skips_prompt`: settings `{"explorer.confirmDelete": false}` → no prompt, trashed.
  - `trash_failure_offers_permanent_delete`: hook returns `Err` → second prompt; `Delete Permanently` → file gone from disk.
  - `delete_permanently_always_asks`: confirm off, `alt-cmd-backspace` → prompt pending.
  - `delete_closes_clean_tabs_keeps_dirty`: project also has `src/lib.rs`; open `src/main.rs` and edit it (dirty), open `src/lib.rs` pinned (clean); delete `src` → `lib.rs` tab closed, `main.rs` tab kept; deliver `Removed` events → `main.rs` state `Deleted`.
  - `selection_moves_to_parent_after_delete`.
- [ ] **Step 2:** Run → FAIL. **Step 3:** Implement. **Step 4:** Run → PASS.
- [ ] **Step 5: Commit** — `feat(explorer): delete to Trash`.

### Task 6: Whole-part check

- [ ] **Step 1:** `cargo test -p lucerna-project -p lucerna-workbench -p lucerna-settings` → PASS; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo deny check`.
- [ ] **Step 2:** In the built app on a scratch folder: New File with `a/b/c.rs`, Rename a dirty open file then save, Delete a folder with open tabs, confirm the file is in the macOS Trash. Put the inline input and error box beside Zed's project panel rename and compare height, border and colours (design-principles §1.1).
- [ ] **Step 3:** Commit fixes with explicit paths.
