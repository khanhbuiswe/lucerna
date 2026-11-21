# Depth limit when scanning the file tree — short design

Date: 2026-11-16. Status: approved. The user chose "do it like Zed".

## Intent

Opening `~` (or a large directory that is not a repo) makes Lucerna scan
the entire tree immediately: it costs CPU, RAM, and reads into every
subdirectory. The user wants it handled like Zed.

## References

Research dated 2026-11-16 (Zed's main branch, VS Code):

- **Zed** (`crates/worktree/src/worktree.rs`, PR #62583, 2026-08): scans
  the whole tree eagerly, but directories **outside a git repo** at depth ≥
  `file_scan_depth` (default 5) are recorded as `UnloadedDir` and not
  scanned into. Such a directory is loaded when the user expands it, or
  when a file inside it is opened. Hitting the limit shows a notice in the
  status bar. When walking up looking for `.git`/`.gitignore`, Zed stops
  at `$HOME`.
- Zed does **not** prevent macOS's permission prompts (TCC): `~/Desktop`,
  `~/Library`, `~/Pictures/Photos Library.photoslibrary` are only 1–2
  levels deep so they still get scanned. Issues #20913 and #48746 are
  still open.
- **VS Code**: the Explorer only reads a directory when it is expanded;
  there is no depth-limit setting. Find in Files and Cmd+P still walk the
  whole tree with ripgrep.

## Lucerna today

- `lucerna-project/src/worktree.rs`: `Worker::scan` walks the whole tree
  with `walk_builder` (`ignore::WalkBuilder`), with no depth limit.
- `walk_builder` turns on `.parents(true)`: reads `.gitignore` in every
  parent directory, all the way up to `/`.
- `search.rs` walks the whole tree itself with the same `walk_builder`.
- The watcher is a recursive `notify` (FSEvents on macOS).

## Approaches considered

1. **Like Zed: scan eagerly, limit depth outside repos** (chosen). Changes
   the model the least: the snapshot is still the full list of what has
   been loaded.
2. Like VS Code: only read on expand. Quick Open and search would need to
   walk separately, a big change; not the direction the user chose.
3. Like (1) plus excluding macOS's TCC directories. Zed does not do it;
   later if the user wants it.

## Design

- **Setting `files.scanDepth`**, default `5`. VS Code has no equivalent
  setting, so the name follows VS Code's `files.*` style (the spec
  writer's decision). `0` or negative: no limit.
- **Scan**: a directory at depth ≥ `files.scanDepth` from the root that is
  not inside any git repo is recorded in the snapshot with the new kind
  `EntryKind::UnloadedDir` and not walked into. A directory inside a git
  repo (the root is inside a repo, or it itself/its parent within the root
  has `.git`) is always scanned in full.
- **Load on demand**:
  - Expanding an `UnloadedDir` in the Explorer: the worker reads that
    directory one level; subdirectories stay `UnloadedDir` if they still
    exceed the limit.
  - Opening a file inside one (CLI, Reveal, go to definition): load the
    directories along the path to that file.
  - Once loaded it is kept until a full rescan; a rescan reloads the
    directories that are expanded.
- **Quick Open, Find in Files**: only see what has been loaded, like Zed.
  Search uses the same depth rule (skipping `UnloadedDir`) instead of
  walking the whole tree.
- **Watcher**: keep the recursive FSEvents; events inside an
  `UnloadedDir` are ignored, except for creating/deleting that directory
  itself.
- **Parent directories**: when the root is under `$HOME`, look for
  `.gitignore` and a git repo in parent directories stopping at `$HOME`
  (including `$HOME`), not going further up.
- **Notice**: the first time a scan hits the limit, the status bar shows
  `Some folders deeper than 5 levels load when expanded`
  (the number follows the setting).
- None of the above depends on the platform.

## Non-goals

- Not preventing macOS permission prompts. Opening `~` can still trigger
  the Photos, Desktop… prompts like Zed. If wanted, do approach (3) in a
  later spec.
- No change to reopening `~` at startup (Zed reopens it too).

## Tests

- `worktree.rs`: a 7-level-deep tree with no `.git` → level 5 and below
  are `UnloadedDir`; the same tree with `.git` → scanned in full;
  `files.scanDepth: 0` → scanned in full; expanding an `UnloadedDir`
  loads exactly one level; loading by file path.
- `search.rs`: a file inside an `UnloadedDir` is not in the results.
- Parent directories: a `.gitignore` above `$HOME` (a fake HOME in the
  test) does not apply.
- Explorer: an `UnloadedDir` shows as a closed folder and can be expanded.

## Left to the implementer

How to mark "inside a git repo" during the parallel walk, how to replace
`.parents(true)` to stop at `$HOME`, and how to show the notice with the
existing components in `ui.rs`/`status_message.rs`.
