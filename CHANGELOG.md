# Changelog

Each release's section below is copied into its GitHub Release page by
`.github/workflows/release.yml`. A release fails if its version has no section here.

## [0.2.1] - 2026-11-19

### Fixed

- Opening a trusted folder no longer flashes "Git is off in Restricted Mode" and a Manage
  Workspace Trust button in a restored Uncommitted Changes tab while the folder's trust is being
  read. The tab shows Loading… until Git starts.

## [0.2.0] - 2026-11-18

### Added

- Image preview. PNG, JPEG, GIF (animated), WebP, BMP, TIFF, ICO and SVG files open in their own
  tab, centred on a checkerboard and shrunk to fit; the status bar shows `W × H • size`. The tab
  reloads when the file changes on disk, also in gitignored folders, and its title is struck
  through when the file is deleted. SVG opens as an image; Cmd+Shift+V, the path bar button or
  Reopen as Text / Reopen as Image Preview switch it to its source and back, asking first when
  the text has unsaved changes. `workbench.editorAssociations` with `"*.svg": "default"` opens SVG
  as text. Files over 20 MB or 100 megapixels, and Git LFS pointers, show a notice with Open as
  Text.
- Multi-cursor like VS Code: Cmd+D and Cmd+K Cmd+D, Shift+Cmd+L, Opt+Cmd+Up/Down, Shift+Opt+I,
  column select (Shift+Opt+Cmd+arrows or Shift+Opt+drag), Opt+click and Opt+drag, paste that
  spreads one line per cursor, and a cursor count in the status bar (`editor.multiCursor*`
  settings).
- Cmd+N (File → New File) opens an Untitled tab. Its tab shows the first line you type, and it
  counts as unsaved once it has text. `files.defaultLanguage` sets its language (Plain Text when
  unset). Untitled tabs with text come back after quitting, as other unsaved changes do (hot exit).
- Save As (Cmd+Shift+S, File → Save As…). Saving an Untitled tab (Cmd+S, or `:w`, `:wq`, `:x` in
  Vim) asks where to save it. The tab becomes the new file and keeps its undo history; with Save
  As on a file, every tab of it moves to the new file and the old file is left as it was.
- Replace in file. Alt+Cmd+F (Edit > Replace) opens the find bar with a Replace row. Enter in the
  Replace box replaces one match and Cmd+Enter replaces all; Shift+Cmd+1 and Alt+Cmd+Enter do the
  same while the bar is open. Each replace is one undo step.
- Regex replace uses VS Code syntax: `$0`/`$&`, `$1`..`$99`, `$$`, `\n`, `\t`, `\\`, and the case
  operators `\u \U \l \L`. The Preserve Case toggle (`AB`) keeps each match's casing.
- Alt+Cmd+C, Alt+Cmd+W, Alt+Cmd+R and Alt+Cmd+P toggle match case, whole word, regex and preserve
  case in the editor's find bar.
- Cmd+Shift+T reopens the last closed tab, in its group and place, with its cursor and scroll;
  press again to go further back. Close Others (Alt+Cmd+T), Close to the Left/Right, Close Saved
  (Cmd+K U) and Close All in Group (Cmd+K W) are in the Command Palette and in a new right-click
  menu on tabs; unsaved files are asked about once for all.
- Replace in Files (⇧⌘H): Replace All across the project, per file and per line, with a preview in the Search tab.

### Changed

- Zoom (Cmd+= / Cmd+-) applies to every editor, diff and window, and to the terminal unless
  `terminal.integrated.fontSize` is set. Cmd+0 resets it, and the status bar shows the new size.

## [0.1.1] - 2026-11-17

### Added

- Vim: macros. `q{a-z}` records and `q{A-Z}` appends; `@{reg}` replays, `@@` repeats the last one,
  and counts work. A whole run undoes in one step. A macro stops at the first command that fails.
- Vim: the `:` command line, opened in Quick Open. It supports `:w :w! :wa :q :q! :qa :wq :x`,
  `:d :y :> :<` with ranges, `:noh`, and `:s/pat/rep/flags` (`vim.gdefault`). On files over 1 MB,
  `:s` runs in the background.
- Vim: highlight on yank (`vim.highlightedyank.enable`, `vim.highlightedyank.duration`).
- `files.scanDepth`: outside git repositories, folders deeper than the limit load when you expand
  them, as in Zed.
- A warning bar for files with bytes that are not valid in their encoding. Lucerna asks before the
  first save replaces those bytes.
- The status bar shows when file watching is off or could not start.

### Changed

- With Vim on, numbers after `:` still mean Go to Line, as in VS Code.
- Go to Line accepts `:-N`, `:N,M` and `:N#M`, as VS Code does.
- A language server's root defaults to the outermost folder with a root marker.
- Development builds use their own bundle id, so they run beside the release app.

### Fixed

- Saving:
  - autosave no longer recreates a deleted file;
  - Keep Mine restarts the `afterDelay` autosave;
  - `files.trimFinalNewlines` empties a file of blank lines;
  - temp file names stay within 255 bytes;
  - on quit, Lucerna waits up to 5 s for saves in flight.
- Explorer and watching:
  - New File with a trailing slash creates a folder;
  - watched folders replaced or recreated keep being watched;
  - Markdown Preview follows a renamed file and keeps its place.
- Search and Quick Open:
  - regex anchors match on CRLF lines;
  - a result past the end of its file opens the last line;
  - recent entries whose path is gone say so.
- `luc`:
  - starts the app without the socket;
  - shares one socket across versions of a build channel;
  - reclaims the socket while the app quits;
  - reports a failed open;
  - `--trace-startup` and `--foreground` refuse a running app.
- Hot exit reopens single-file windows, even when the file was deleted.
- The terminal's exit notice is in English.

## [0.1.0] - 2026-11-16

First release, for macOS on Apple silicon.

[0.2.1]: https://github.com/khanhbuiswe/lucerna/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/khanhbuiswe/lucerna/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/khanhbuiswe/lucerna/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/khanhbuiswe/lucerna/releases/tag/v0.1.0
