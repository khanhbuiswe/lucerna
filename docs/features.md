# Features

What Lucerna does today. For the short version, see the [README](../README.md).

## Opening folders and files

```sh
luc .                     # open the current folder
luc ~/code/my-project     # open a folder
luc src/main.rs           # open a file on its own
```

If you open Lucerna from Finder or the Dock, it reopens every window you had open when it quit
(`"window.restoreWindows"`: `"all"`, or `"folders"`, `"one"`, `"preserve"`).
Set `"window.restoreWindows": "none"` to get the Welcome page with your recent folders instead
(and `"workbench.startupEditor": "none"` for a folder picker). Open Folder, Open Recent
and Close Folder (Cmd+K F) switch the window in place. A file opens on its own:
the sidebar lists only that file, and neither its folder nor its git repository is
scanned. Open the folder to work on the whole project. A folder outside a git repository
(say `~/Downloads`) is scanned `"files.scanDepth"` levels deep (5 by default); deeper folders
load when you expand them.

Lucerna also appears in Finder's "Open With" menu for folders, text files and source files. A file
inside the folder that is already open opens as a tab; anything else opens in a new window. To make
Lucerna the default app for a file type, select a file in Finder, then choose File → Get Info →
Open with → Lucerna → Change All. Apps that open files in your default app then use Lucerna, for
example "Open in → Default app" in Claude Desktop.

## What it does

- **Follows your agent.** When a file changes on disk, the open view reloads in about 70 ms and keeps
  its scroll position, syntax colours and folds. Atomic writes, `git checkout` of thousands
  of files, and deleted or recreated files are all handled. Files the agent just changed are
  marked in the tree for 10 seconds.
- **Fast on big repos.** In a release build, an 80,000-file repo shows its tree in about 60 ms and finishes scanning in about 1.2 s. Idle
  memory is about 170 MB with a medium repo and 10 tabs open. See
  [`docs/benchmarks.md`](benchmarks.md) for the method and the three-editor table, and
  [`docs/perf-checklist.md`](perf-checklist.md) for the tree and scan figures.
- **VS Code muscle memory.**
  - Cmd+P opens Quick Open, which also takes `>` for commands and `:` for go to line.
  - Cmd+Shift+F searches the project, Cmd+F finds in the file, Cmd+\ splits the editor.
  - Settings and keys live in `settings.json` and `keybindings.json`, with `when` clauses and per-language overrides.
  - Changes apply as soon as you save them.
- **Minimal UI.**
  - Compact chrome: an integrated title bar that shows the git branch, and panel toggles in the status bar.
  - Flat tabs with preview tabs, a path bar, indent guides and thin scrollbars you can drag or click.
  - A Welcome page with recent folders and files (Ctrl+R opens Open Recent anywhere).
  - Lilex and IBM Plex Sans fonts, coloured Seti file icons (VS Code's default set) in the tree and on tabs (`workbench.editor.showIcons`), Lucide UI icons, and the Lucerna Dark and Lucerna Light themes, which follow the system's light or dark mode.
- **See what the agent changed (Git).** Read-only and fast: changed, added and untracked files are
  coloured in the tree (folders too); the gutter marks added, modified and deleted lines against
  `HEAD`; Alt+F5 / Shift+Alt+F5 (and `]c` / `[c` in Vim) jump between changes. Click a gutter mark
  or run *Git: Open Changes* for a file's diff, *Git: Open Changes (Uncommitted)* for every
  uncommitted change in one tab, and *Git: Open Changes Since Branch* to review a whole branch.
  Updates follow the agent's writes and `git` commands. Lucerna never stages or commits; git runs
  only in trusted folders.
- **Integrated terminal.** Ctrl+` toggles a terminal panel at the bottom (Ctrl+Shift+` opens
  another tab), so the agent's CLI can run next to the code. Colours follow the theme, Vietnamese
  IME input works, full-screen apps (Claude Code, vim) run, and Cmd+click opens a file path or URL
  the terminal printed. Settings use VS Code's `terminal.integrated.*` names.
- **Markdown preview.** Cmd+Shift+V toggles a `.md` file between source and a rendered preview;
  Cmd+K V opens a preview to the side that follows the active Markdown file. Tables, task lists,
  images and code blocks render natively, and the preview follows edits and agent writes.
- **Explorer context menu.** Right-click (or Shift+F10) a file or folder: New File, New Folder,
  Rename (open tabs follow), Delete to Trash or Delete Permanently, Copy Path, Copy Relative Path,
  Reveal in Finder, Open to the Side and Find in Folder.
- **Syntax highlighting** with tree-sitter for Rust, TypeScript/TSX, JavaScript, Python, Go, JSON,
  Markdown, TOML, YAML, Bash, HTML and CSS. More languages come from extensions.
- **Code intelligence over LSP.** Go to definition, references, hover, diagnostics and outline from
  the language server you already have on your `PATH`. A built-in table covers about 40 servers
  (rust-analyzer, gopls, basedpyright, typescript-language-server, clangd, intelephense, ruby-lsp,
  jdtls, sourcekit-lsp, zls and more); add or override servers under `"lsp"` in `settings.json`.
  Run `luc lsp health`, or click "Show All Languages…" in the ⚡ status bar popover, to see which
  servers are installed and how to install the rest.
- **Extensions from the Zed registry.** Install themes, icon themes and languages from the existing
  Zed extension ecosystem. Extensions are built locally, with no call to `api.zed.dev`.
- **Large-file safe.** Files over 20 MB or 300k lines skip highlighting. A 5 MB minified line
  renders without freezing. Binary files and files in other encodings (UTF-16, Latin-1, …) are detected.
- **Editing.** Typing (with IME), cursor movement, delete and word delete, Enter that keeps the
  indent, Tab and outdent, cut, copy and paste (a whole line when nothing is selected), and
  undo and redo grouped like VS Code, plus cursor undo. Multi-cursor like VS Code: Cmd+D and
  Cmd+K Cmd+D, Shift+Cmd+L, Opt+Cmd+Up/Down, Shift+Opt+I, column select (Shift+Opt+Cmd+arrows or
  Shift+Opt+drag), Opt+click and Opt+drag, paste that spreads one line per cursor, and a status bar count
  (`editor.multiCursor*` settings). Find's whole-word option follows `editor.wordSeparators`, so `$foo` and `foo-bar` match as words. Save, Save All, Save Without Formatting
  and Revert; `files.trimTrailingWhitespace`, `files.insertFinalNewline`, `files.trimFinalNewlines`,
  `files.eol` and `files.autoSave`. Writes are atomic and keep the file's encoding, BOM and line
  ending. Tabs show a dirty dot, and a conflict dot when the file changed on disk. Saving over
  a changed file asks first, and closing a tab or window with unsaved changes prompts. Hot exit: unsaved changes are
  backed up about a second after you stop typing. Quitting by any means (Cmd+Q, Dock Quit, Log
  Out, a crash) keeps them without asking, and they come back when the folder or file is
  reopened; a file that changed on disk meanwhile asks before you overwrite it.
  `files.hotExit: "off"` restores the old asking behaviour.
- **Smart typing.** Brackets and quotes close themselves, type over their closer, delete as a
  pair and wrap a selection. They follow the language and stay out of strings and comments.
  Enter indents after brackets (and by the extension's `indents.scm`), and typing a closer
  outdents. The indentation of a file is detected, and the status bar shows `Spaces: N` or
  `Tab Size: N`. Toggle line and block comments (languages with only block comments wrap each
  line), move, copy, delete, insert, indent, outdent and join lines, trim trailing whitespace,
  and expand or shrink the selection by syntax.
- **Vim (optional).** Set `"vim.enabled": true` for Vim keys: motions, search,
  scrolling, marks, operators with motions and text objects, registers, `.` repeat, Visual mode editing, macros and `:` commands (`:w`, `:q`, `:%s`…). It is off by default. See [Vim mode](vim.md).
- **Vietnamese / IME input** works in every text field (Telex, VNI).
