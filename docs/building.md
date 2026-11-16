# Building and installing from source

## From the disk image (macOS)

```sh
scripts/make-dmg.sh              # builds Lucerna.app, then target/release/Lucerna.dmg
open target/release/Lucerna.dmg  # drag Lucerna into Applications, choose Replace
```

`make-dmg.sh` packs the build it just made (`--no-build` packs the app already built, as the
release workflow does), and ejects a Lucerna disk image left mounted from an earlier run. Eject the image after you install, so macOS doesn't keep the copy on it.

The app is ad-hoc signed and not notarized. The first time you open it, right-click it and choose **Open**.

## Install straight from source

```sh
scripts/bundle-app.sh --install  # puts Lucerna.app in /Applications and `luc` in ~/.local/bin
```

Set `LUC_BIN_DIR` to install `luc` somewhere else. Make sure that directory is on your `PATH`.

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
| `lucerna-editor` | Editing core: edits, undo, smart typing, three-way merge with agent writes |
| `lucerna-syntax` | tree-sitter languages, highlighting, folds, brackets |
| `lucerna-project` | Directory scan, file watcher, fuzzy finder, project search |
| `lucerna-settings` | Layered `settings.json`, keymap and `when` clauses |
| `lucerna-theme` | Theme loading (Zed theme format) and the built-in Lucerna themes |
| `lucerna-vim` | Vim state machine: motions, operators, text objects, registers. No GPUI |
| `lucerna-lsp` | Language server client and the server table |
| `lucerna-extension` | Zed extension registry, install and build |
| `lucerna-git` | Read-only git: status, `HEAD` blobs, hunks (runs the `git` binary) |
| `lucerna-terminal` | Terminal emulation and PTY, on `alacritty_terminal` |
| `lucerna-workbench` | The whole UI. It is the only crate that depends on GPUI. |
| `lucerna` | The `luc` binary |

Design documents are in [`docs/superpowers/specs/`](superpowers/specs/), and research notes on how VS Code
and Zed do things are in [`docs/research/`](research/). Before designing a feature, read
[`docs/design-principles.md`](design-principles.md).

## Status and known limits

Pre-release, heading for a first public build (0.1). Lucerna reads, edits and saves
code; follows an agent's writes and merges them into unsaved text (a conflict bar with Reload /
Keep Mine when they overlap); shows git changes in the tree, the gutter and diff tabs; has an
integrated terminal, a Markdown preview, an Explorer context menu and optional Vim keys.
Not done yet: multi-cursor, completion, rename and format (Phase 4.3, 4.5, 4.6), Vim macros and
`:` commands, a settings UI. Known limits: quitting from the Dock or at logout cannot show the
save prompt (a GPUI limitation at the pinned revision), there is no hot exit, and builds are not
notarized yet. It runs on **macOS** today. **Linux and Windows** are planned, and GPUI already
supports both. See [Roadmap](#roadmap) below.

## Roadmap

| Phase | What it adds | Status |
|---|---|---|
| 1 | Core viewer: tree, tabs, highlighting, Quick Open, search, live reload | ✅ Done |
| 2 | Zed extensions: themes, icon themes, languages | ✅ Done |
| 3 | Code intelligence over LSP: go to definition, references, hover, diagnostics, outline | ✅ Done |
| 3b | WASM host for Zed extensions: language servers shipped inside `extension.wasm`. PHP, Ruby, Elixir, Vue and others already work through the language server table when their server is installed | Planned |
| 4 | Editing: typing, saving, undo, multi-cursor, handling conflicts with agent writes | 4.1 (editing core), 4.2 (smart typing) and 4.4 (agent conflicts) done; 4.3 (multi-cursor), 4.5 (completion, snippets) and 4.6 (rename, format) planned |
| 5 | Git status, gutter and diff tabs; integrated terminal; Markdown preview; Explorer context menu; settings UI | Done except the settings UI |
| V1 | Vim navigation: motions, search, scroll, marks, visual copy | ✅ Done |
| V2 | Vim editing | V2a (operators, text objects, registers, `.`) done; V2b (macros, `:` commands) planned |
| — | Public release: notarized build, published benchmarks | In progress |
| — | Linux and Windows support, using GPUI's Wayland/X11 and DirectX backends | Planned |

## Licenses

Lucerna is licensed under either of [MIT](../LICENSE-MIT) or [Apache-2.0](../LICENSE-APACHE), at your option.

Bundled third-party assets keep their own licenses. These include the Lilex and IBM Plex fonts (OFL-1.1),
Lucide icons (ISC), Seti file icons (the SVGs from seti-ui, MIT, Jesse Weed, and VS Code's Seti mapping, MIT, Microsoft), GitHub Primer colours used by the Lucerna themes (MIT), tree-sitter grammars,
`alacritty_terminal` and GPUI (Apache-2.0).
See [`THIRD_PARTY_NOTICES.md`](../THIRD_PARTY_NOTICES.md) for a summary. The app carries the full
license text of every bundled crate and asset: open it with Help › Third-Party Licenses, or the Licenses
button in About. `scripts/bundle-app.sh` writes it with `scripts/gen-licenses.py`.
