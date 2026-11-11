<p align="center">
  <img src="assets/brand/app-icon.svg" width="96" alt="Lucerna icon">
</p>

<h1 align="center">Lucerna</h1>

<p align="center">
  A fast, minimal code editor for reading what your AI agent wrote.<br>
  Written in Rust. No built-in AI, no bloat.
</p>

---

Your agent writes the code in Claude, Codex or a CLI such as Claude Code. You open Lucerna to
read it: browse the project, jump between files and see what the agent just changed. Lucerna
opens instantly, follows changes on disk as they happen, and keeps VS Code's keyboard shortcuts
and `settings.json`, so there is nothing new to learn.

```
Claude / Codex (app or CLI) writes  →  Lucerna shows  →  you review
```

> **Status:** Phase 2 of the roadmap is complete. Lucerna is a **viewer** for now, and editing
> comes later. It runs on **macOS** today. **Linux and Windows** are planned, and GPUI already
> supports both. See [Roadmap](#roadmap).

## Why Lucerna

**Why not VS Code?** VS Code runs on Electron. Lucerna is written in Rust and draws on the GPU
with [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui), the UI framework from
the Zed project. In a release build the first frame takes about 150 ms, idle memory is about
110 MB, and an 80,000-file repo shows its tree in about 60 ms
(see [`docs/perf-checklist.md`](docs/perf-checklist.md)). You keep VS Code's shortcuts, setting
names and `keybindings.json` format.

**Why not Zed?** Zed is fast too, but it is an AI-first editor: it ships its own agent panel,
edit predictions, collaboration and account sign-in. Lucerna leaves the agent to the tool you
already use and stays small. It has no AI features, no telemetry and no account. It is fast,
simple and does one job well: showing you the code.

**Built for one flow.** Reading code is the first priority. Every feature is there to help you
follow, navigate and understand what the agent wrote, and anything that does not serve that is
left out.

## Features

- **Follows your agent.** When a file changes on disk, the open view reloads in about 70 ms and keeps
  its scroll position, syntax colours and folds. Atomic writes, `git checkout` of thousands
  of files, and deleted or recreated files are all handled. Files the agent just changed are
  marked in the tree for 10 seconds.
- **Fast on big repos.** In a release build, the first frame takes about 150 ms. An 80,000-file repo shows
  its tree in about 60 ms and finishes scanning in about 1.2 s. Idle memory is about 110 MB.
  See [`docs/perf-checklist.md`](docs/perf-checklist.md).
- **VS Code muscle memory.**
  - Cmd+P opens Quick Open, which also takes `>` for commands and `:` for go to line.
  - Cmd+Shift+F searches the project, Cmd+F finds in the file, Cmd+\ splits the editor.
  - Settings and keys live in `settings.json` and `keybindings.json`, with `when` clauses and per-language overrides.
  - Changes apply as soon as you save them.
- **Minimal UI.**
  - Compact chrome: an integrated title bar that shows the git branch, and panel toggles in the status bar.
  - Flat tabs with preview tabs, a path bar, indent guides and thin scrollbars.
  - Lilex and IBM Plex Sans fonts, Lucide icons, and the One Dark and One Light themes.
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
- **Vim navigation (optional).** Set `"vim.enabled": true` for read-only Vim keys: motions, search,
  scrolling, marks and visual selection to copy. It is off by default. See [Vim mode](#vim-mode).
- **Vietnamese / IME input** works in every text field (Telex, VNI).

## Install

Builds are macOS-only for now. Linux and Windows packages will come with cross-platform support
(see [Roadmap](#roadmap)).

### From the disk image (macOS)

```sh
scripts/make-dmg.sh              # builds Lucerna.app, then target/release/Lucerna.dmg
open target/release/Lucerna.dmg  # drag Lucerna into Applications, choose Replace
```

`make-dmg.sh` always packs the build it just made, and ejects a Lucerna disk image left mounted
from an earlier run. Eject the image after you install, so macOS doesn't keep the copy on it.

The app is ad-hoc signed and not notarized. The first time you open it, right-click it and choose **Open**.

### Install straight from source

```sh
scripts/bundle-app.sh --install  # puts Lucerna.app in /Applications and `luc` in ~/.local/bin
```

Set `LUC_BIN_DIR` to install `luc` somewhere else. Make sure that directory is on your `PATH`.

## Usage

```sh
luc .                     # open the current folder
luc ~/code/my-project     # open a folder
luc src/main.rs           # open a file (its git repo becomes the workspace)
```

If you open Lucerna from Finder or the Dock, it asks which folder to open. A loose file in your home
folder opens on its own, without scanning the whole home folder.

Lucerna also appears in Finder's "Open With" menu for folders, text files and source files. A file
inside the folder that is already open opens as a tab; anything else opens in a new window. To make
Lucerna the default app for a file type, select a file in Finder, then choose File → Get Info →
Open with → Lucerna → Change All. Apps that open files in your default app then use Lucerna, for
example "Open in → Default app" in Claude Desktop.

### Keyboard shortcuts

| Keys | Action |
|---|---|
| Cmd+P | Go to file |
| Cmd+Shift+P | Command palette |
| Ctrl+G | Go to line |
| Cmd+F | Find in file |
| Cmd+Shift+F | Search the project |
| Cmd+B | Toggle sidebar |
| Cmd+Shift+E | Explorer |
| Cmd+Shift+X | Extensions |
| Cmd+\ | Split the editor |
| Cmd+1…8 | Focus editor group N |
| Cmd+W | Close tab |
| Ctrl+Tab | Recent tabs |
| Cmd+K Cmd+T | Pick a theme |
| Cmd+, | Open `settings.json` |
| Alt+Z | Toggle word wrap |
| Cmd+= / Cmd+- | Zoom in / out |
| Cmd+Q | Quit |

Rebind any of these in `~/.config/lucerna/keybindings.json`, which uses the same format as VS Code.

### Settings

| File | Path |
|---|---|
| User settings | `~/.config/lucerna/settings.json` |
| User keybindings | `~/.config/lucerna/keybindings.json` |
| User themes | `~/.config/lucerna/themes/*.json` (Zed theme format) |
| Workspace settings | `<project>/.lucerna/settings.json` |
| Logs | `~/Library/Logs/Lucerna/` |

The settings use VS Code's names, for example:

```jsonc
{
  "editor.fontFamily": "Lilex",
  "editor.fontSize": 14,
  "editor.lineHeight": 1.6,
  "editor.wordWrap": "off",
  "editor.minimap.enabled": false,
  "workbench.colorTheme": "One Dark",
  "files.exclude": { "**/dist": true },
  "[go]": { "editor.tabSize": 8 }
}
```

### Vim mode

Vim navigation is built in and off by default. It takes effect as soon as you save the setting.
The status bar shows the mode, and the cursor becomes a block. Lucerna is still a viewer, so
there is no insert mode or editing yet.

| Group | Keys |
|---|---|
| Motions | `h j k l`, `w b e ge` (and `W B E gE`), `0 ^ $`, `gg G`, `{ }`, `%`, `H M L`, `f F t T` with `;` `,`, counts |
| Search | `/` and `?` (open the find widget), `n N`, `* #` |
| Scroll | `Ctrl+d` `Ctrl+u`, `Ctrl+f` `Ctrl+b`, `zz zt zb` |
| Marks | `m{a-z}`, `` `x ``, `'x`, `` `` `` and `''` |
| Visual | `v`, `V`, `Ctrl+v`, `o`, `y` to copy, `Esc` |
| Folds | `za zc zo` |

In Normal and Visual mode the arrow keys act as `h j k l`, and Enter moves to the first
non-blank character of the next line.

| Setting | Default | Meaning |
|---|---|---|
| `vim.enabled` | `false` | Turn Vim navigation on. `vim_mode` is read as an alias when this is unset |
| `vim.ignorecase` | `true` | Ignore case for `*` and `#` |
| `vim.wrapscan` | `true` | Searches wrap around the end of the file |
| `vim.startofline` | `true` | `gg`, `G`, `H`, `M`, `L` go to the first non-blank character |
| `vim.scroll` | `0` | Lines for `Ctrl+d` / `Ctrl+u`; `0` means half a page |
| `editor.cursorSurroundingLines` | `0` | Lines kept around the cursor when it moves |

## Extensions

Lucerna installs extensions from the Zed registry
([`zed-industries/extensions`](https://github.com/zed-industries/extensions)). Supported types:

- themes
- icon themes
- languages, as tree-sitter WASM grammars with their queries and injections

Snippets are validated but not used yet. Everything an extension provides works right after
it is installed, with no restart. Extensions are built on your machine. Lucerna does not call
`api.zed.dev` and does not distribute any extension.

- **In the app:** press Cmd+Shift+X, or click the Extensions button in the status bar.
  - Search, then choose Install, Update or Uninstall.
  - After a theme installs, a toast offers **Set Theme**.
  - To pick an icon theme, use *Preferences: File Icon Theme*.
- **From the terminal**, with no window needed:

  ```sh
  luc ext search <query>
  luc ext list [--installed] [--updates]
  luc ext install <id>... | --dev <dir>
  luc ext remove <id>...
  luc ext update [<id>...]
  ```

What extensions need at run time:

- `git`, which comes with the Xcode Command Line Tools (`xcode-select --install`).
- wasi-sdk 34, which is downloaded the first time a grammar is built. It is about 100 MB and is stored under
  `~/Library/Application Support/Lucerna/extensions/toolchain/`. To use a copy you already have, set `WASI_SDK_PATH`.
- About 1.2 GB of free disk space while an extension is fetched and built.

Installed extensions live in `~/Library/Application Support/Lucerna/extensions/`. To use another
location, set `LUCERNA_EXTENSIONS_DIR`. Each install writes a log to `logs/<id>.log`.

## Building from source

Requirements: macOS 13 or later, Rust (the toolchain is pinned in `rust-toolchain.toml`), and the Xcode
Command Line Tools. A full Xcode install is not needed.

```sh
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"   # only if cargo comes from Homebrew's rustup
brew install cmake                                  # needed to build wasmtime
cargo run -p lucerna -- .                           # debug build
```

`target/debug/luc` runs without an app bundle, so macOS shows "luc" in the menu bar. Use
`scripts/bundle-app.sh --debug` to wrap the dev build in `Lucerna.app`.

Tests and checks (CI runs the same ones):

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
cargo deny check
```

### Workspace layout

| Crate | What it does |
|---|---|
| `lucerna-text` | Rope buffer, encoding detection, reload diff |
| `lucerna-syntax` | tree-sitter languages, highlighting, folds, brackets |
| `lucerna-project` | Directory scan, file watcher, fuzzy finder, project search |
| `lucerna-settings` | Layered `settings.json`, keymap and `when` clauses |
| `lucerna-theme` | Zed theme loading |
| `lucerna-vim` | Vim navigation: motions, search, marks, visual mode. No GPUI |
| `lucerna-workbench` | The whole UI. It is the only crate that depends on GPUI. |
| `lucerna` | The `luc` binary |

Design documents are in [`docs/superpowers/specs/`](docs/superpowers/specs/), and research notes on how VS Code
and Zed do things are in [`docs/research/`](docs/research/). Before designing a feature, read
[`docs/design-principles.md`](docs/design-principles.md).

## Roadmap

| Phase | What it adds | Status |
|---|---|---|
| 1 | Core viewer: tree, tabs, highlighting, Quick Open, search, live reload | ✅ Done |
| 2 | Zed extensions: themes, icon themes, languages | ✅ Done |
| 3 | Code intelligence over LSP: go to definition, references, hover, diagnostics, outline | ✅ Done |
| 3b | WASM host for Zed extensions: language servers shipped inside `extension.wasm`. PHP, Ruby, Elixir, Vue and others already work through the language server table when their server is installed | Planned |
| 4 | Editing: typing, saving, undo, multi-cursor, handling conflicts with agent writes | Planned |
| 5 | Git gutter and diff view, integrated terminal, settings UI | Planned |
| V1 | Vim navigation: motions, search, scroll, marks, visual copy | ✅ Done |
| V2 | Vim editing (after phase 4) | Planned |
| — | Linux and Windows support, using GPUI's Wayland/X11 and DirectX backends | Planned |

## License

Lucerna is licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.

Bundled third-party assets keep their own licenses. These include the Lilex and IBM Plex fonts (OFL-1.1),
Lucide icons (ISC), the One theme (MIT), tree-sitter grammars and GPUI (Apache-2.0).
See [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md).
