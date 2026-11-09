<p align="center">
  <img src="assets/brand/app-icon.svg" width="96" alt="Lucerna icon">
</p>

<h1 align="center">Lucerna</h1>

<p align="center">
  A fast, read-first code viewer for macOS.<br>
  A slimmer VS Code with a Zed-style UI, built for people who write code through CLI agents.
</p>

---

When Claude Code, Codex or another agent writes the code, you mostly *read* it: you browse
the project, jump between files, and watch what the agent just changed. Lucerna is built
for that. It opens instantly, follows changes on disk as they happen, and keeps VS Code's
keyboard shortcuts and `settings.json`, so there is nothing new to learn.

Lucerna is written in Rust on [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui)
(the GPU UI framework behind Zed). It has no AI features and no telemetry.

> **Status:** Phase 2 of the roadmap is complete. Lucerna is a **viewer**, so editing comes later.
> See [Roadmap](#roadmap).

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
- **Zed-style UI.**
  - A compact chrome with the traffic lights inside the window and the git branch in the title bar.
  - Flat tabs with preview tabs, a path bar, indent guides and thin scrollbars.
  - Lilex and IBM Plex Sans fonts, Lucide icons, and the One Dark and One Light themes.
- **Syntax highlighting** with tree-sitter for Rust, TypeScript/TSX, JavaScript, Python, Go, JSON,
  Markdown, TOML, YAML, Bash, HTML and CSS. More languages come from extensions.
- **Zed extensions.** Install themes, icon themes and languages from the Zed extension registry.
  They are built locally, with no call to `api.zed.dev`.
- **Large-file safe.** Files over 20 MB or 300k lines skip highlighting. A 5 MB minified line
  renders without freezing. Binary files and files in other encodings (UTF-16, Latin-1, …) are detected.
- **Vietnamese / IME input** works in every text field (Telex, VNI).

## Install

### From the disk image

```sh
scripts/bundle-app.sh            # builds Lucerna.app (release)
scripts/make-dmg.sh              # target/release/Lucerna.dmg
open target/release/Lucerna.dmg  # drag Lucerna into Applications
```

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
| 3 | Code intelligence over LSP: go to definition, references, hover, diagnostics, outline | Planned |
| 4 | Editing: typing, saving, undo, multi-cursor, handling conflicts with agent writes | Planned |
| 5 | Git gutter and diff view, integrated terminal, settings UI | Planned |
| V1 / V2 | Vim mode: motions first (after phase 2), then editing (after phase 4) | Planned |

## License

Lucerna is licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.

Bundled third-party assets keep their own licenses. These include the Lilex and IBM Plex fonts (OFL-1.1),
Lucide icons (ISC), the One theme (MIT), tree-sitter grammars and GPUI (Apache-2.0).
See [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md).
