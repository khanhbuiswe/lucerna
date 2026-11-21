# Open a single file like Zed — short design

Date: 2026-11-13. Status: Approved.

## Intent

Opening a single file (Finder, `luc file`, Open File…, Open Recent) currently
opens the whole parent folder (for example `~/Downloads`) or the whole git
repo containing the file, so the sidebar lists the whole folder. Users want it
like Zed: opening a file shows only that file.

## Current state

`resolve_target` (`crates/lucerna/src/target.rs`) picks the root for a file:
the nearest git repo, otherwise the parent folder. Single-file mode (R27: no
scan, no watching, sidebar and Quick Open contain only that file, Search
disabled) already exists but only turns on when the parent folder is `$HOME`
or `/`.

## References

- Zed (`crates/workspace/src/workspace.rs`, `OpenVisible`): opening a file
  into an empty project creates a worktree containing only that file
  (`OpenVisible::All`); opening another file into a project that already has
  a visible worktree makes that file hidden (`OpenVisible::None`). It does
  not look for a git repo and does not take the parent folder.
- VS Code: `code file` opens a window with no folder; the Explorer says no
  folder is open. It also does not look for a git repo.

## Approaches considered

1. **Every single file goes into single-file mode** (chosen). Reuses R27,
   little code, matches Zed and VS Code.
2. Keep the git repo rule, open only files outside a repo as single files.
   Dropped: the user chose to do it like Zed; differs from both reference
   editors.
3. A new mode with Zed-style hidden files. Dropped: single-file mode is
   enough, this is left for later.

## Design

- `resolve_target`: a file always returns `single_file: true`, `root` = parent
  folder. Remove the `.git` lookup and the `$HOME`/`/` condition. Folders
  still open as before.
- Sidebar: in single-file mode do not draw the root row (the folder); only
  the file's row remains, like Zed. The title bar shows the file name. (The
  first approved version said "the root row shows the file name"; changed
  while writing the plan because it would repeat the name twice.)
- Opening more files: Open File… inside the app opens a tab in this window
  (as now); Finder/`luc` opens a new window (as now, `open_requests::route`).
- Restricted Mode: trust still follows the folder containing the file.
- Recent: a single file is recorded as `RecentKind::File` (already exists).
- Tests: update the git repo rule tests in `target.rs` and `tests/cli.rs`; add
  tests that a file in a plain folder (like `~/Downloads`) and a file in a
  repo both open as single files; tests for no root row and the title being
  the file name.

## Not doing

- A second file shown hidden in the same window like Zed.
- Changing the folder in the current window (a separate job for the Open
  Folder session).

## Left to the implementer

How to put the file name into the sidebar root label and the title; test
names.
