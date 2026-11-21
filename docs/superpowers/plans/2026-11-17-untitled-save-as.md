# New Untitled File and Save As Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** ⌘N opens an Untitled tab; ⌘S / ⇧⌘S saves it (or any file) to a path chosen in a dialog; Untitled goes through closing a tab, quitting, Save All and hot exit without losing text.

**Architecture:** Untitled is an ordinary document in `Documents` with a relative pseudo key `untitled:Untitled-N`. A new module `untitled.rs` holds every pure rule (recognition, numbering, tab name, suggested name). Every place that touches disk or an outside service checks `untitled::is_untitled` and skips. Save As writes to the destination, then uses the existing rename path (`Workbench::paths_renamed`) to move the document and tabs to the real path.

**Tech Stack:** Rust, GPUI (test platform: `simulate_new_path_selection`, `did_prompt_for_new_path`, `simulate_prompt_answer`), crate `lucerna-workbench`, `lucerna-settings`.

**Spec:** `docs/superpowers/specs/2026-11-17-untitled-save-as-design.md`

## Global Constraints

- Cargo: `export PATH="/opt/homebrew/opt/rustup/bin:$PATH"`; `scripts/prune-target.sh` before every build/test; do not set `CARGO_TARGET_DIR`; builds share `target/` in the main checkout.
- Only `lucerna-workbench` uses gpui. Make no macOS-specific assumptions outside platform code (dialogs use the common GPUI API).
- Every string shown is English. Fixed names: `Untitled-N`; the command `workbench.action.files.newUntitledFile` "File: New Untitled Text File" (`cmd+n`, menu "New File"); `workbench.action.files.saveAs` "File: Save As..." (`shift+cmd+s`, menu "Save As…"); the setting `files.defaultLanguage` defaults to `""`.
- Tab name: the first non-empty line, trimmed of whitespace, at most 40 characters, with `…` added when cut; empty → `Untitled-N`.
- Suggested name when saving: the tab name without `/ \ : * ? " < > |` and control characters, trimmed of whitespace; empty → `Untitled-N`; no extension added.
- ⌘N does not run on Welcome / with no window (already right thanks to `WELCOME_COMMANDS` / `NO_WINDOW_COMMANDS`; only a test is needed to keep it so).
- No file whose name starts with `untitled:` is created on disk, in any test.
- Commit: stage each file by path; trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. Saving Untitled to a path that is **open in another tab**: there are no longer two tabs with the same path, and the Untitled tab is kept — test `save_untitled_over_open_file` (Task 5).
2. The Save As write fails (a read-only directory): the tab is still Untitled, still dirty, no text lost — test `save_as_write_error_keeps_untitled` (Task 5).
3. Type into Untitled then delete everything: closing the tab does not ask, and hot exit keeps no empty backup — test `emptied_untitled_closes_silently` (Task 6).
4. Two Untitled tabs, close `Untitled-1`, then ⌘N: the new tab is `Untitled-1` — test `untitled_numbers_are_reused` (Task 4).
5. Untitled when the folder is a git repo with a `.gitignore` / a real watcher running: no `untitled:` path reaches git or the watcher — test `untitled_never_reaches_git_or_watcher` (Task 3).

---

### Task 1: The `untitled` module and keys that pass through intact

**Files:**
- Create: `crates/lucerna-workbench/src/untitled.rs` (register `pub mod untitled;` in `lib.rs`, in alphabetical order)
- Modify: `crates/lucerna-workbench/src/workspace_state.rs:123` (`absolute_path`)

**Interfaces:**
- Produces:
  - `pub const PREFIX: &str = "untitled:";`
  - `pub fn is_untitled(path: &Path) -> bool` — true when `path` is relative, has exactly one component, and starts with `PREFIX`.
  - `pub fn path(n: u32) -> Arc<Path>` — `untitled:Untitled-{n}`.
  - `pub fn number(path: &Path) -> Option<u32>`.
  - `pub fn name(path: &Path) -> String` — `Untitled-N` (the part after `PREFIX`).
  - `pub fn next_number(used: impl IntoIterator<Item = u32>) -> u32` — the smallest number from 1 not yet used.
  - `pub fn label(text_first_lines: &str) -> Option<String>` — the tab name from the first non-empty line (rule in Global Constraints), `None` when no line has text.
  - `pub fn suggested_file_name(label: &str) -> Option<String>` — the suggested name, `None` when empty after filtering.
- `workspace_state::absolute_path(root, path)` returns `path` unchanged when `is_untitled(path)`.

- [ ] **Step 1: Unit tests in `untitled.rs`**

```rust
#[test] fn recognizes_only_relative_single_component() {
    assert!(is_untitled(Path::new("untitled:Untitled-1")));
    assert!(!is_untitled(Path::new("/tmp/untitled:Untitled-1")));
    assert!(!is_untitled(Path::new("dir/untitled:x")));
    assert!(!is_untitled(Path::new("src/main.rs")));
}
#[test] fn numbers() {
    assert_eq!(&*path(3), Path::new("untitled:Untitled-3"));
    assert_eq!(number(&path(3)), Some(3));
    assert_eq!(name(&path(3)), "Untitled-3");
    assert_eq!(next_number([]), 1);
    assert_eq!(next_number([1, 3]), 2);
    assert_eq!(next_number([2, 1]), 3);
}
#[test] fn labels() {
    assert_eq!(label(""), None);
    assert_eq!(label("\n   \n"), None);
    assert_eq!(label("\n  fn main() {}\nx").as_deref(), Some("fn main() {}"));
    let long = "a".repeat(50);
    assert_eq!(label(&long).unwrap(), format!("{}…", "a".repeat(40)));
}
#[test] fn suggested_names() {
    assert_eq!(suggested_file_name("a/b:c*?.rs").as_deref(), Some("abc.rs"));
    assert_eq!(suggested_file_name(" <>|\"\\ "), None);
}
```

And in the `workspace_state.rs` tests: `absolute_path(Path::new("/r"), Path::new("untitled:Untitled-1"))` == `Path::new("untitled:Untitled-1")`.

- [ ] **Step 2: Run, see it fail** — `cargo test -p lucerna-workbench --lib untitled workspace_state` → compile error.
- [ ] **Step 3: Write `untitled.rs` per Interfaces; fix `absolute_path`.** `label` counts by `char`, not by byte.
- [ ] **Step 4: Run, see it pass** — same command → PASS.
- [ ] **Step 5: Commit** — `feat: untitled document naming rules`

---

### Task 2: `Documents` creates and holds Untitled documents

**Files:**
- Modify: `crates/lucerna-workbench/src/documents.rs` — `open_view` (~643), `is_dirty` (1075), `dirty_paths` (1266), `save` (1305), `revert`/`revert_now` (1679/1698), `on_worktree_event` Rescanned (~1992), `open_from_backup` (2140), `restore` (2949), `language_for` (2825)
- Modify: `crates/lucerna-settings/assets/default-settings.json` (add `"files.defaultLanguage": ""` next to the `files.*`), `docs/settings.md`
- Test: `crates/lucerna-workbench/tests/untitled.rs` (new; copy `init`/`editor`/`settle`/`run` from `tests/save.rs:28-133`)

**Interfaces:**
- Consumes: Task 1.
- Produces:
  - `pub fn Documents::new_untitled(&mut self, cx: &mut Context<Self>) -> Arc<Path>` — picks `untitled::next_number` from the Untitled keys currently in `docs`, creates an empty document in the Ready state, with no job, the language following `files.defaultLanguage` (`LanguageRegistry::language_by_name`, not found → Plain Text), and emits `Opened`. Returns the key.
  - `Documents::open(path)` with an Untitled key that already exists returns a new view of that document; an Untitled key that does not exist yet creates an empty one (used for restore), and never reads the disk.
  - `is_dirty(untitled)` = the buffer is not empty; `dirty_paths()` includes the non-empty Untitled documents.
  - `save(untitled, ..)` returns `Ok(false)` at once, writing nothing (the path that saves Untitled is Task 5).
  - `revert(untitled)`: does nothing. `on_worktree_event`: ignores Untitled keys. `language_for(untitled)`: follows `files.defaultLanguage`.
  - `open_from_backup` with an Untitled header: does not read the disk, builds the document from the backup's text, dirty.

- [ ] **Step 1: Tests in `tests/untitled.rs`**

```rust
#[gpui::test] fn new_untitled_is_empty_and_clean(cx: &mut TestAppContext)
// docs.new_untitled() == "untitled:Untitled-1"; the second time == Untitled-2;
// snapshot empty; is_dirty == false; dirty_paths() empty.
#[gpui::test] fn untitled_with_text_is_dirty(cx)
// type "x" into the view → is_dirty == true, dirty_paths() == [untitled:Untitled-1].
#[gpui::test] fn untitled_never_touches_disk(cx)
// save(untitled) → Ok(false); revert(untitled); send WorktreeEvent::Rescanned;
// after settle: no entry in root (fs::read_dir) has a name containing "untitled:".
#[gpui::test] fn default_language_applies(cx)
// settings { "files.defaultLanguage": "rust" } → language_of(untitled) == Some("rust");
// "nope" → Plain Text (None, or the plaintext id like a .txt file today).
```

- [ ] **Step 2: Run, see it fail** — `cargo test -p lucerna-workbench --test untitled` → compile error.
- [ ] **Step 3: Implement per Interfaces.** Check `untitled::is_untitled(&key)` at the top of each function listed in Files.
- [ ] **Step 4: Run, see it pass** — the same command, together with `--test save --test hot_exit` (nothing broken) → PASS.
- [ ] **Step 5: Commit** — `feat: untitled documents in Documents`

---

### Task 3: Keep Untitled away from LSP, git, the watcher, symbols, preview, recent, workspace state

**Files:**
- Modify: `language_servers/sync.rs:77` (Opened, and Edited/Saved/Closed/LanguageChanged of an Untitled key); `git/wiring.rs:282` (`git_document_event`); `trust.rs:560`; `symbols/source.rs:175`; `workbench.rs:1471` (`refresh_watched_open_files`) and `1498` (`open_tab_paths`); `markdown_preview/commands.rs:354` (`follow_active`); `quick_open/mod.rs:42` (`RecentFiles::record`); `workbench.rs:1880` (recent when opening a file); `editor_groups/model.rs:853` (`saved_path`: Untitled does not go into the saved tab list); `explorer/view.rs:1118` (`sync_active_tab`: no reveal)
- Test: `crates/lucerna-workbench/tests/untitled.rs`

**Interfaces:**
- Consumes: Task 1 (`is_untitled`), Task 2 (`new_untitled`).
- Produces: no new API; each place above returns early when `untitled::is_untitled(path)`.

- [ ] **Step 1: Test**

```rust
#[gpui::test] fn untitled_never_reaches_lsp(cx)
// use tests/support/lsp.rs::lsp_workbench; settings files.defaultLanguage = the fake server's language;
// new_untitled + type text → count(didOpen) == 0, count(didChange) == 0.
#[gpui::test] fn untitled_never_reaches_git_or_watcher(cx)
// tests/support/git.rs::repo; open Untitled, type; workbench.open_tab_paths() does not contain the Untitled path;
// the git store does not track any path starting with "untitled:".
#[gpui::test] fn untitled_not_in_workspace_state_or_recent(cx)
// save_workspace_state → the state has no Untitled tab; RecentFiles has no Untitled.
```

Look for more places that touch the disk by document path (`grep -n "fs::\|std::fs" src/` on the functions that take a tab/document path); for any place found, add a guard and note it in the ledger.

- [ ] **Step 2: Run, see it fail** — `cargo test -p lucerna-workbench --test untitled untitled_never untitled_not` → FAIL.
- [ ] **Step 3: Add a guard at each place in Files.**
- [ ] **Step 4: Run, see it pass** — `cargo test -p lucerna-workbench --test untitled --test lsp_sync --test git_gutter --test workspace_state` → PASS.
- [ ] **Step 5: Commit** — `feat: keep untitled documents away from disk services`

---

### Task 4: The ⌘N command, tab name, icon, path bar, menu

**Files:**
- Modify: `workbench.rs` (a new command in `default_registry` ~3463; `pub fn new_untitled_file(&mut self, window, cx)`); `editor_groups/model.rs:165` (`Tab::title`) or `editor_groups/view.rs:809-815` (override the name for Untitled with `untitled::label` of the snapshot, else `untitled::name`); `editor_groups/view.rs:152` (`tab_icon_source`: Untitled → the icon of a `.txt` file); `editor_groups/view.rs:578,1015` (path bar: `Untitled-N`); `app_menu.rs:236-257` (File → "New File" right above "New Window"; fix the tests at `993`, `1011` for the new order); `keys.rs:189` (add a `cmd+n` line); `crates/lucerna-settings/assets/default-keybindings.json:133-141`
- Docs: `docs/keyboard-shortcuts.md`, `docs/features.md` (the Editing paragraph), `CHANGELOG.md` `[Unreleased]` → `### Added`
- Test: `tests/untitled.rs`, `tests/app_menu.rs`

**Interfaces:**
- Consumes: Task 2 `Documents::new_untitled`.
- Produces: `Workbench::new_untitled_file(&mut self, window: &mut Window, cx: &mut Context<Self>)` — creates the key with `new_untitled`, opens a pinned tab (`OpenMode::Pinned`) in the active group through `open_file` (not through `absolute_path` / recent), and focuses the view.

- [ ] **Step 1: Test**

```rust
#[gpui::test] fn cmd_n_opens_untitled_tab(cx)
// cx.simulate_keystrokes("cmd-n") → the active tab has path "untitled:Untitled-1", pinned, focus in the code view;
// the displayed tab title == "Untitled-1".
#[gpui::test] fn untitled_title_follows_first_line(cx)
// type "\n  hello world\nsecond" → title "hello world"; delete everything → "Untitled-1".
#[gpui::test] fn untitled_numbers_are_reused(cx)
// ⌘N twice (1, 2); close tab 1 (empty, no prompt); ⌘N → "untitled:Untitled-1".
```

`tests/app_menu.rs`: the File menu starts with `["New File", "New Window", …]`; `welcome_disables_workbench_items` adds `assert!(item(file, "New File").is_disabled())`.

- [ ] **Step 2: Run, see it fail** — `cargo test -p lucerna-workbench --test untitled cmd_n untitled_title untitled_numbers --test app_menu` → FAIL.
- [ ] **Step 3: Implement per Files/Interfaces.** Take the display name of the Untitled tab from the `Documents` snapshot (`snapshot_of`) — read only the first 1–2 lines, do not walk the whole buffer.
- [ ] **Step 4: Run, see it pass** — the same command, plus `--lib keys` → PASS.
- [ ] **Step 5: Commit** — `feat: Cmd+N opens an untitled tab`

---

### Task 5: Save As (Untitled and real files)

**Files:**
- Modify: `documents.rs` (the new write-to-destination function); `save.rs` (the command `workbench.action.files.saveAs` in `register_commands` ~635; `save` 636 goes to Save As when the active one is Untitled; the function that asks where to save); `workbench.rs` (`pub fn save_as`; uses `paths_renamed` 1278); `vim_ex.rs:51-109` (`:w!`, `:wq`, `:x` on Untitled go to Save As; `:wq`/`:x` close only once saved); `app_menu.rs` (File → "Save As…" right below "Save"); `keys.rs` (a `shift+cmd+s` line); `default-keybindings.json`
- Docs: `docs/keyboard-shortcuts.md`, CHANGELOG
- Test: `tests/untitled.rs`, `tests/save.rs` (Save As of a real file), `tests/vim_command_line.rs`

**Interfaces:**
- Consumes: Task 1 (`suggested_file_name`, `label`, `name`), Task 2, Task 4.
- Produces:
  - `pub fn save_dialog_start(wb: &Workbench, path: &Path) -> (PathBuf, String)` (trong `save.rs`) — the starting directory and the suggested name: Untitled → (`root`, or the file's parent directory when `is_single_file`) and `suggested_file_name(label)` or `name(path)`; a real file → (the parent directory, the file name).
  - `pub fn Documents::write_as(&mut self, from: &Path, to: &Path, window: &mut Window, cx: &mut Context<Self>) -> Task<Result<FileStamp, WriteError>>` — writes the current text of `from` to `to` through the existing atomic write flow (participants follow `to`'s settings; Untitled is written as UTF-8 without BOM; a real file keeps its encoding / BOM / line ending); no conflict check (the dialog already asked about Replace). Does not change the key. (`FileStamp` = the stamp type that `finish_save` uses.)
  - `pub fn Workbench::save_as(&mut self, path: Arc<Path>, window: &mut Window, cx: &mut Context<Self>) -> Task<bool>` — `cx.prompt_for_new_path(dir, Some(name))`; Cancel / error → `false`, nothing changes. Choosing exactly the current `path` → like a normal Save. Otherwise: `write_as`; on success (a) if `to` is open: close the tabs of `to` and drop its old document; (b) `paths_renamed(path, to)` (the document, every tab, preview, nav); (c) mark clean with the stamp just written (a function shared with `finish_save`); return `true`. On a write error: report it like a normal save (the existing `status_message`/prompt), do not change the key, return `false`.

- [ ] **Step 1: Test**

```rust
#[gpui::test] fn save_untitled_asks_for_path_and_becomes_file(cx)
// ⌘N, type "fn main() {}\n"; cx.simulate_new_path_selection(|dir| { assert_eq!(dir, root); Some(root.join("main.rs")) });
// ⌘S → did_prompt_for_new_path(); disk("main.rs") == "fn main() {}\n"; tab path == root/main.rs (absolute);
// is_dirty == false; language_of == Some("rust"); cmd-z can still undo the typing.
#[gpui::test] fn save_untitled_suggests_first_line(cx)
// type "notes: todo\n" → the closure receives the suggested name "notes todo" (check through a test hook if GPUI lets us read it; otherwise
// unit-test save_dialog_start directly).
#[gpui::test] fn save_untitled_cancel_keeps_it(cx)
// simulate_new_path_selection(|_| None) → the tab is still "untitled:Untitled-1", dirty, root has no new file.
#[gpui::test] fn save_untitled_over_open_file(cx)
// a.txt is open in another tab; save Untitled as a.txt → exactly 1 tab has path a.txt, its content = Untitled's.
#[gpui::test] fn save_as_write_error_keeps_untitled(cx)
// destination in a directory with chmod 0o555 → the tab is still Untitled, dirty, text intact.
#[gpui::test] fn untitled_in_two_groups_both_become_file(cx)
// Untitled open in two groups (split) → after saving, both tabs have the destination path.
```

`tests/save.rs`:

```rust
#[gpui::test] fn save_as_moves_file_tab_and_keeps_old_file(cx)
// a.txt is "old" on disk, edited to "new" (unsaved); ⇧⌘S choosing b.txt →
// disk(a.txt) == "old"; disk(b.txt) == "new"; every tab of a.txt is now b.txt; not dirty; undo is kept.
#[gpui::test] fn save_as_same_path_is_save(cx)
```

`tests/vim_command_line.rs`: `:w` on Untitled calls the dialog; `:wq` with Cancel does not close the tab; `:wq` closes when the save succeeds.

- [ ] **Step 2: Run, see it fail** — `cargo test -p lucerna-workbench --test untitled --test save --test vim_command_line save_` → FAIL.
- [ ] **Step 3: Implement per Interfaces.** The `files.save` command and `:w`, when the active path is Untitled, call `Workbench::save_as`.
- [ ] **Step 4: Run, see it pass** — the same command, plus `--lib keys --test app_menu` → PASS.
- [ ] **Step 5: Commit** — `feat: Save As, and saving untitled files`

---

### Task 6: Close, quit, Save All, autosave, hot exit with Untitled

**Files:**
- Modify: `save.rs:165` (`save_each`: Untitled goes through `Workbench::save_as` one at a time; Cancel when closing a tab / quitting → stop the whole flow; in Save All → skip that one, go on); `save.rs:469-605` (autosave: `auto_save_paths`, `auto_save_document`, `auto_save_event` skip Untitled); `editor_groups/view.rs:215,249` (closing: an empty Untitled does not ask — already so thanks to `is_dirty`, only a test is needed); `workbench.rs:1941,2038` (quitting with `hotExit: off` uses the new `save_each`; `auto_save_paths` at quit skips Untitled); `hot_exit/tracker.rs:137` (`backup_of`: Untitled → `mtime: None, size: None`; an empty Untitled → delete the backup); `documents.rs:2140,2949` (restoring Untitled: does not read the disk); `workbench.rs:645-650` (a restored Untitled opens at the end of the first group through `open_restored`, keeping the exact `Untitled-N` from the header)
- Test: `tests/untitled.rs`, `tests/hot_exit.rs`

**Interfaces:**
- Consumes: Task 2, Task 5 (`Workbench::save_as`).
- Produces: `save_each` keeps its current signature (`fn save_each(documents, paths, &mut AsyncWindowContext) -> bool`) but takes an extra way to call Save As (a `WeakEntity<Workbench>` parameter if needed — note it in the ledger if the signature changes).

- [ ] **Step 1: Test**

```rust
#[gpui::test] fn emptied_untitled_closes_silently(cx)
// ⌘N, type "x", delete → ⌘W has no pending prompt; the tab closes.
#[gpui::test] fn closing_untitled_with_text_asks(cx)
// type "x"; ⌘W → pending prompt; "Save" + simulate_new_path_selection(Some(root/x.txt)) → the file exists, the tab closes;
// another time "Save" + choose None → the tab stays open; "Don't Save" → the tab closes, no file.
#[gpui::test] fn autosave_skips_untitled(cx)
// files.autoSave "afterDelay", autoSaveDelay 10; type; wait → still Untitled, dirty, no new file.
#[gpui::test] fn save_all_asks_for_each_untitled(cx)
// two Untitled with text + a dirty a.txt; saveAll; choose a path for the 1st, None for the 2nd →
// a.txt is saved; Untitled-1 becomes a file; Untitled-2 is still dirty.
```

`tests/hot_exit.rs` (using `Project`, `open_with_data_dir`, `quit`, `reopen`, `tab_paths`, `text_of`):

```rust
#[gpui::test] fn reopen_restores_untitled(cx)
// ⌘N, type "draft"; quit; reopen → there is a tab "untitled:Untitled-1" at the end of the first group, text "draft", dirty.
#[gpui::test] fn empty_untitled_leaves_no_backup(cx)
// ⌘N, type then delete; quit → backups() empty; reopen → no Untitled tab.
#[gpui::test] fn hot_exit_off_quit_asks_for_untitled_path(cx)
// files.hotExit "off"; Untitled with text; confirm_quit → "Save All" + choose None → the quit is cancelled.
```

- [ ] **Step 2: Run, see it fail** — `cargo test -p lucerna-workbench --test untitled --test hot_exit` → FAIL.
- [ ] **Step 3: Implement per Files.**
- [ ] **Step 4: Run, see it pass** — `cargo test -p lucerna-workbench` (the whole crate) and `cargo clippy -p lucerna-workbench --all-targets -- -D warnings` and `cargo fmt --check` → PASS, clean. (If `lsp_settings_per_window` / `open_to_side_uses_second_group` are flaky because of the real rust-analyzer, rerun just that file and note it in the ledger.)
- [ ] **Step 5: Mark done** — `docs/roadmap.md`: `- [x] New file (⌘N) and Save As`; spec: status "implemented (branch …); manual check §6 not done".
- [ ] **Step 6: Commit** — `feat: untitled files through close, quit, Save All and hot exit`

---

### Manual check (after Task 6)

Run the debug build: ⌘N → type → ⌘S → save `foo.rs` in the project → it has colors, LSP, and the Explorer shows the file; ⇧⌘S a file → the old file is intact; quit with an unsaved Untitled → on reopening it is intact; Welcome: the New File menu is dimmed.
