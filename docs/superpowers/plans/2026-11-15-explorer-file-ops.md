# Explorer Cut/Copy/Paste, Duplicate, Open Preview Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The Explorer context menu and keys gain Cut / Copy / Paste (with VS Code " copy" naming), Duplicate (Zed-style, then rename the copy) and Open Preview for markdown files.

**Architecture:** Pure naming and no-overwrite copy live in `lucerna-project::fs_ops`. An Explorer clipboard (`{op, path}`) lives on the Workbench. Paste/Duplicate run I/O in the background, record own changes first, and on a move reuse `Workbench::paths_renamed` so tabs/documents follow. `markdown.showPreview` learns `args.path`.

**Tech Stack:** Rust, GPUI (`lucerna-workbench`), `lucerna-project`.

**Spec:** `docs/superpowers/specs/2026-11-15-explorer-file-ops-design.md` (builds on `docs/superpowers/specs/2026-11-13-explorer-context-menu-design.md`).

## Global Constraints

- Only `lucerna-workbench` uses gpui; no macOS-only code outside `cfg`.
- Never overwrite: every write uses `create_new` / `fs_ops::rename` (no-replace); collisions get " copy" names.
- UI thread never waits on I/O; own operations are not decorated (`expect_own_changes` before, `forget_own_changes` on error).
- Naming (exact): file `"{stem} copy.{ext}"`, then `"{stem} copy 2.{ext}"`, `" copy 3"`…; dir / no-extension `"{name} copy"`, `"{name} copy 2"`…; dotfile `.env` → `.env copy`.
- IDs/keys (macOS, `when: filesExplorerFocus && !textInputFocus`): `filesExplorer.cut` cmd+x, `filesExplorer.copy` cmd+c, `filesExplorer.paste` cmd+v, `filesExplorer.duplicate` cmd+d, `filesExplorer.cancelCut` escape (`&& explorerResourceCut`). Labels: `Cut`, `Copy`, `Paste`, `Duplicate`, `Open Preview`.
- Status copy (invented, keep exact): `Unable to paste '{name}': {err}`, `Cannot paste '{name}' into itself.`
- Build: shared cargo target only (CLAUDE.local.md). Never set `CARGO_TARGET_DIR` / `--target-dir`. `export PATH="/opt/homebrew/opt/rustup/bin:$PATH"`. If another session holds the cargo lock, wait. Run `scripts/prune-target.sh` before tests.
- Commit with explicit paths, trailer exactly `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- GPUI tests that open files and then press keys: settings `{"session.trustAllWorktrees": true, "lucerna.lsp.enabled": false}` and `window.activate_window()` (see tests/explorer_edit.rs).

## Review Focus

- Paste a folder into itself or its descendant → refused with the status copy, nothing written (Task 1 `copy_refuses_into_itself`, Task 3 `paste_folder_into_itself_refused`).
- Cut a dirty open file and paste elsewhere → tab follows, edits kept, saving writes the new path (Task 3 `cut_paste_moves_dirty_tab`).
- Collision chain: `a.rs`, `a copy.rs` exist → paste gives `a copy 2.rs` (Task 1 `paste_name_increments`, Task 3 `paste_twice_names_copy_2`).
- Symlink inside a copied folder stays a symlink, target not followed (Task 1 `copy_dir_keeps_symlinks`).
- Paste with an empty clipboard / from the root menu after cut of a deleted file → no-op, no error dialog (Task 3 `paste_vanished_source_reports_status`).

---

### Task 1: fs_ops naming and copy

**Files:** Modify `crates/lucerna-project/src/fs_ops.rs`; Test `crates/lucerna-project/tests/fs_ops.rs`.

**Interfaces (produces):**
- `pub fn paste_name(dir: &Path, name: &str, is_dir: bool, exists: impl Fn(&Path) -> bool) -> (PathBuf, Range<usize>)` — first free target in `dir`; the range is the byte range in the final file name to select for renaming (`0..` end of `" copy N"`, or `0..stem_len` when no suffix was needed).
- `pub fn copy_entry(from: &Path, to: &Path) -> io::Result<()>` — file: `create_new` + copy bytes + copy permissions; dir: create `to` (fail `AlreadyExists` if present) then recurse; symlink: recreate the link (`std::os::unix::fs::symlink` / Windows `symlink_file`/`symlink_dir`) without following; refuses (`InvalidInput`) when `to` is inside `from`.
- `pub fn is_within(path: &Path, ancestor: &Path) -> bool` (component-wise, `path == ancestor` counts).

- [ ] Step 1: failing tests — `paste_name_increments` (`a.rs` free → `a.rs`, range `0..1`; `a.rs` taken → `a copy.rs`, range `0..6`; `a.rs`,`a copy.rs` taken → `a copy 2.rs`, `0..8`), `paste_name_dir_and_dotfile` (`src` → `src copy`; `.env` → `.env copy`; `a.tar.gz` → `a.tar copy.gz`), `copy_file_never_overwrites`, `copy_dir_recursive`, `copy_dir_keeps_symlinks` (`#[cfg(unix)]`), `copy_refuses_into_itself`, `is_within_cases` (`/a/b` within `/a`; `/ab` not within `/a`).
- [ ] Step 2: `cargo test -p lucerna-project --test fs_ops` → FAIL. Step 3: implement. Step 4: PASS; clippy, fmt.
- [ ] Step 5: commit `feat(project): paste naming and no-overwrite copy`.

### Task 2 (controller): merge main

Another session is changing `explorer/menu.rs` and `explorer/commands.rs` on main (Open in Integrated Terminal, `terminal.explorerKind`). Before Task 3, merge `main` into this branch (`git merge main`, never rebase) once that work is committed; resolve nothing by hand without reading both sides.

### Task 3: Explorer clipboard, Cut / Copy / Paste

**Files:** Modify `crates/lucerna-workbench/src/workbench.rs` (clipboard field + context key `explorerResourceCut`), `explorer/commands.rs` (commands), `explorer/menu.rs` (items), `explorer/view.rs` (dim cut row), `crates/lucerna-settings/assets/default-keybindings.json`, `src/keys.rs` test table; Test new `crates/lucerna-workbench/tests/explorer_clipboard.rs`.

**Interfaces:**
- Consumes Task 1; existing `command_target`, `expect_own_changes`, `forget_own_changes`, `paths_renamed`, `Documents::saves_done_under`, `ExplorerView::select`/`reveal_path`, `open_path(…, OpenMode::Pinned)`, `status_message::show`.
- Produces `pub enum ClipOp { Cut, Copy }`, `pub struct ExplorerClipboard { pub op: ClipOp, pub path: Arc<Path> /* absolute */ }`, `Workbench::explorer_clipboard(&self) -> Option<&ExplorerClipboard>`; `pub(crate) fn paste_into(wb, dest_dir_abs, cx, window) ` shared with Duplicate as `fn transfer(wb, src_abs, dest_dir_abs, op, after: AfterTransfer, window, cx)` where `enum AfterTransfer { SelectAndOpen, Rename(Range<usize>) }`.
- Menu: insert group `Cut`, `Copy`, `Paste` (disabled_when clipboard empty), `Duplicate` after the Find-in-Folder group; Cut/Paste/Duplicate hidden for Root and single-file; Copy hidden for Root.
- Paste target dir: args.path/selection if dir or root, else its parent. Move into the folder that already holds it → no-op. Cut + successful paste clears the clipboard; Copy keeps it.
- Cut row: name in `text_muted` while it is the clipboard's Cut source; debug selector `explorer-row-cut-{path}`.

- [ ] Step 1: failing tests — `copy_paste_into_folder` (copy `a.rs`, select `src`, cmd+v → `src/a.rs`, selected, opened pinned, not decorated), `paste_twice_names_copy_2`, `paste_on_file_uses_its_folder`, `cut_paste_moves_file` (source gone, clipboard empty after), `cut_paste_moves_dirty_tab` (edit open `a.rs`, cut, paste into `src` → tab path `src/a.rs`, still dirty; cmd+s writes it), `cut_into_same_folder_is_noop`, `paste_folder_into_itself_refused` (status copy, nothing written), `paste_disabled_when_clipboard_empty` (menu item disabled), `cut_row_is_dimmed_and_escape_cancels`, `paste_vanished_source_reports_status`, `root_and_single_file_menus` (no Cut/Paste/Duplicate).
- [ ] Step 2: run → FAIL. Step 3: implement. Step 4: PASS (`cargo test -p lucerna-workbench --test explorer_clipboard --test explorer_view --test explorer_commands --test explorer_edit`), fmt, clippy.
- [ ] Step 5: commit `feat(explorer): cut, copy and paste files`.

### Task 4: Duplicate

**Files:** `explorer/commands.rs`, `explorer/view.rs` (`start_rename_with(range: Range<usize>)`), keybindings + keys.rs row; Test `tests/explorer_clipboard.rs`.

- Duplicate = `transfer(src, parent_of_src, Copy, AfterTransfer::Rename(range))`; never touches the clipboard; after the copy appears, starts the inline rename on the copy with the input selection = `range` from `paste_name`.
- [ ] Step 1: failing tests — `duplicate_file_then_renames` (dup `a.rs` → `a copy.rs` exists, rename edit open on it, selected range `0..6`), `duplicate_folder`, `duplicate_keeps_clipboard`, `cmd_d_duplicates`.
- [ ] Step 2–4: FAIL → implement → PASS (+ fmt, clippy). Step 5: commit `feat(explorer): duplicate`.

### Task 5: Open Preview from the Explorer

**Files:** `crates/lucerna-workbench/src/markdown_preview/commands.rs` (`markdown.showPreview` honours `args.path`), `explorer/menu.rs` (item after Open to the Side for markdown files — language from `AppState.languages.language_for_file`, `files.associations` included); Test `tests/explorer_commands.rs` or the markdown preview tests.

- With `args.path`: open that file with `FileView::Preview`, pinned (use the existing `open_file` path of `EditorGroups`); without: unchanged behaviour.
- [ ] Step 1: failing tests — `open_preview_from_menu_opens_markdown_preview` (right-click `README.md` → item present → click → active tab is the preview of `README.md`), `no_open_preview_for_rust_file`.
- [ ] Step 2–4: FAIL → implement → PASS. Step 5: commit `feat(explorer): Open Preview for markdown files`.

### Task 6: Whole-branch check

- [ ] `cargo test --workspace --no-fail-fast`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all --check`, `cargo deny check` (shared target). Update the main spec's "Claude's decisions" only if behaviour diverged from this spec.
