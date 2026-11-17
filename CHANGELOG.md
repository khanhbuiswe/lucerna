# Changelog

Each release's section below is copied into its GitHub Release page by
`.github/workflows/release.yml`. A release fails if its version has no section here.

## [Unreleased]

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

[0.1.1]: https://github.com/khanhbuiswe/lucerna/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/khanhbuiswe/lucerna/releases/tag/v0.1.0
