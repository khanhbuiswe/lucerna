# Lucerna

A read-only code viewer for macOS, written in Rust with GPUI: a slimmer VS Code
with Zed-style UI, built for people who write code through CLI agents.
The binary is `luc`. Licensed MIT OR Apache-2.0 (see `THIRD_PARTY_NOTICES.md`).

## Building

```
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"   # if cargo comes from Homebrew's rustup
brew install cmake                                  # wasmtime's build needs it
cargo build -p lucerna
target/debug/luc .
```

## Extensions

Lucerna installs extensions from the Zed registry
(`zed-industries/extensions`): themes, icon themes and languages (tree-sitter
WASM grammars with their queries and injections). Snippets are validated but not
used yet. Everything an extension provides works right after installing, with no
restart. Extensions are built on your machine; Lucerna does not call
`api.zed.dev` and does not distribute any extension.

- **In the app:** press Cmd+Shift+X (or click the Extensions button in the
  status bar) to open the Extensions page in an editor tab. Search, then
  Install, Update or Uninstall. After installing a theme, a toast offers
  "Set Theme". Pick an icon theme with *Preferences: File Icon Theme*.
- **From the terminal** (no window needed):

  ```
  luc ext search <query>
  luc ext list [--installed] [--updates]
  luc ext install <id>... | --dev <dir>
  luc ext remove <id>...
  luc ext update [<id>...]
  ```

Requirements at run time:

- `git`, which comes with the Xcode Command Line Tools (`xcode-select --install`).
- wasi-sdk 34, downloaded automatically the first time a grammar is built
  (about 100 MB, stored under `~/Library/Application Support/Lucerna/extensions/toolchain/`).
  Set `WASI_SDK_PATH` to use an existing copy.
- About 1.2 GB of free disk space while fetching and building.

Installed extensions live in `~/Library/Application Support/Lucerna/extensions/`
(override with `LUCERNA_EXTENSIONS_DIR`); each install writes a log to `logs/<id>.log`.
