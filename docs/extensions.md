# Extensions

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
