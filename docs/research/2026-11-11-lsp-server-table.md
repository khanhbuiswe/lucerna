# Research: language server table from data (Helix, Zed, VS Code)

- **Date:** 2026-11-11
- **Status:** research notes, the basis for "a language server for every language" (Phase 3b / after Phase 4). The "What Lucerna chooses" section (§9) is only an **initial proposal**, to be settled in the spec after discussion with the user.
- **Sources:**
  - Helix: `helix-editor/helix` branch `master` taken on 2026-11-11 (the latest tagged release is 25.07.1, 2025-07-18); docs https://docs.helix-editor.com/languages.html, wiki https://github.com/helix-editor/helix/wiki/Language-Server-Configurations. Helix is MPL-2.0.
  - Zed: the local checkout `~/.cargo/git/checkouts/zed-a70e2ad075855582/b5cd310/` (`docs/src/configuring-languages.md`, `assets/settings/default.json`, `crates/settings_content/src/project.rs`, `crates/project/src/lsp_store/vue_language_server_ext.rs`). These crates are **GPL-3**: we only learn the design and copy no code.
  - VS Code: https://code.visualstudio.com/api/language-extensions/language-server-extension-guide and the generic client extensions. Sublime LSP: https://lsp.sublimetext.io/client_configuration/.
  - Wu: https://github.com/dstrml/wu. Zed terms: https://zed.dev/terms.
  - Each server's upstream documentation (URLs at the end of the file).
- Items not verified against the original source on the day are marked **(unverified)**.
- Context: `crates/lucerna-lsp/src/settings.rs` already has Zed's `lsp.<server>.{binary{path,arguments,env}, initialization_options, settings}` shape for the 5 prebuilt adapters (rust-analyzer, gopls, pyright/basedpyright, typescript-language-server, clangd). This document answers the question: how to add many more languages without writing a Rust adapter for each server.

## 1. Summary

| Topic | Helix | Zed | VS Code / Sublime | Lucerna leans toward |
|---|---|---|---|---|
| Defining a new server in config | Yes: `[language-server.<name>]` + `[[language]]` | **No**: `lsp.<name>` only tweaks servers registered by an adapter or extension | VS Code core has none; generic extensions use their own keys; Sublime has `clients` | Yes, following Zed's JSON shape + a few extra keys |
| Binding a server to a language | `language-servers` in `[[language]]`, supports `only-features` / `except-features` | `languages.<L>.language_servers` with `!name` and `"..."` | Sublime: `selector` (scope) | Keep Zed-style `languages.<L>.language_servers` |
| Config merge | Default, user, project; merged by `name`, arrays replaced wholesale | Layered settings; the user's `language_servers` array fully replaces the default | Depends on the extension | Merge objects by key, replace arrays, `!`/`...` only for `language_servers` |
| Server configuration | One `config` block used for both `initializationOptions` and `workspace/configuration` | `initialization_options` and `settings` are separate | Separate (client option and settings) | Two separate blocks like Zed |
| Trusting project files | Master (2026-06): asks for trust for `.helix/languages.toml`, bound to a SHA-256 hash | Restricted Mode | Workspace Trust | A project file may not set `command`/`binary.path`/`env` until trusted |
| Install check | `hx --health [lang]`: only `which(command)` | No equivalent view | No | An "LSP health" view that probes `--version`, with install hints |
| Defaults in the binary | `languages.toml` (MPL-2.0) | Rust adapters + extensions | Extensions | Lucerna's own table, written from upstream docs, not copying Helix's file |
| Pulling extensions from `api.zed.dev` | Wu still does | Yes | No | **No**: conflicts with ToS §2.2 and §2.4(b) |

## 2. Helix

Sources: the docs `languages.html`; `languages.toml` (5,657 lines), `helix-loader/src/lib.rs` (`merge_toml_values`), `helix-loader/src/config.rs` (`user_lang_config`), `helix-loader/src/workspace_trust.rs`, `helix-term/src/health.rs`, `helix-term/src/application.rs`, `helix-lsp/src/client.rs`.

### 2.1 `[language-server.<name>]`

In the default file, every server lives in one `[language-server]` table as an inline table (for example `bash-language-server = { command = "bash-language-server", args = ["start"] }`). Users can write `[language-server.<name>]` as usual.

| Key | Type | Meaning |
|---|---|---|
| `command` | string | Binary name (looked up in `$PATH`) or a path. Required. |
| `args` | string[] | argv after the command. |
| `config` | any TOML | Sent as `initializationOptions`; also the source for answering `workspace/configuration`. The `format` sub-table is used as options when formatting. |
| `timeout` | seconds | Maximum time of one request, default 20. |
| `environment` | table of string to string | Environment variables for the process. |
| `required-root-patterns` | string[] (glob) | The server only runs if at least one glob matches in the workspace. Used for cases like tailwind running only when a config file exists. |

There is no per-server `roots`, `file-types` or `language-id`; those live on the language. This lets one server be reused for several languages (`vscode-css-language-server` for css/scss/less).

### 2.2 `[[language]]` (the LSP-related part)

| Key | Meaning |
|---|---|
| `name` | Helix's language id, also the merge key. |
| `language-servers` | An ordered array. Each element is a server name (string) or `{ name = "...", only-features = [...] }` / `{ name = "...", except-features = [...] }` (the two keys are mutually exclusive). |
| `roots` | Marker files for finding the LSP root, searched upward from the current file to the workspace root. Master also accepts globs (C# uses `["*.slnx", "*.sln", "*.csproj"]`). The **outermost** directory with a marker wins: `find_lsp_workspace` overwrites the result each time it meets a marker while walking up, stopping at the workspace root or at a directory in `workspace-lsp-roots`. Lucerna has followed this since 2026-11-17, see `docs/superpowers/specs/2026-11-17-lsp-outermost-root-design.md`. |
| `language-id` | `TextDocumentItem.languageId`. Defaults to `name`; set only when different (`javascriptreact`, `typescriptreact`, `terraform`, `terraform-vars`, `dockercompose`). |
| `file-types` | An array of file suffixes (`"rs"`) or `{ glob = "..." }`. Globs are checked before suffixes. |
| `workspace-lsp-roots` | Subdirectories treated as separate LSP roots (monorepos). Set under `[editor]` or per language. |

The feature names of `only-features` / `except-features` (exact): `format`, `goto-definition`, `goto-declaration`, `goto-type-definition`, `goto-reference`, `goto-implementation`, `signature-help`, `hover`, `document-highlight`, `completion`, `code-action`, `workspace-command`, `document-symbols`, `workspace-symbols`, `diagnostics`, `rename-symbol`, `inlay-hints`.

How it coordinates (per the docs): for most features, the first server in the list that supports the request wins. `diagnostics`, `code-action`, `completion`, `document-symbols` and `workspace-symbols` alone are sent to every enabled server and the results are merged. Example in the default table: Erlang lists `erlang-ls` and `elp`, both with `except-features = ["document-symbols", "workspace-symbols"]`.

### 2.3 Three-layer merge

`user_lang_config` loads the `languages.toml` compiled into the binary (`include_bytes!`), then overlays `~/.config/helix/languages.toml` and (if trusted) `<workspace>/.helix/languages.toml`, each layer with `merge_toml_values(left, right, 3)`. The later layer wins. Rules:

- Table + table, depth > 0: union of keys; a key present in both is merged recursively with depth-1.
- Array + array, depth > 0: for each right-hand element that has a `name` key, the left-hand element with the same `name` is dropped and the merged result is appended at the end. Elements with no matching name are appended. That is why `[[language]] name = "php"` in the user file merges into the default php entry instead of creating a second one.
- Depth 0 or other value pairs (string, number, string array): the right-hand value replaces.

Consequences (depth 3 counted from the document root):
- `[language-server.foo]`: each key of the server is merged; `args` is replaced as a whole array; sub-tables of `config` deeper than depth 3 are replaced, not merged.
- `[[language]]` merges by `name`; the arrays `language-servers`, `roots`, `file-types` are replaced as a whole (there is no append syntax, and no `!`/`...` like Zed). To add a server you must rewrite the whole list.
- The project applies last, so project wins over user.

### 2.4 Trust

Version 25.07.1 loads `.helix/languages.toml` unconditionally. Master adds workspace trust (PR https://github.com/helix-editor/helix/pull/15857, merged 2026-06-20):

- `TrustQuery` covers: language server, debug adapter, local `.helix/` config, git config.
- `ImplicitTrustLevel`: `None` (ask about everything), `Servers` (default: LSP/DAP from the **global** config run silently because the binaries are installed by the user and found through PATH; the workspace's local config still needs explicit trust), `Insecure`.
- Trust is tied to the SHA-256 hash of the `.helix/` tree (direnv-style): changing the content after trusting makes it "stale" and it must be approved again, so pulling in a malicious branch cannot silently change `command`.
- There is a `prompt` flag and globs of trusted paths. `hx --health` runs with `WorkspaceTrust::fully_trusted()`.

### 2.5 `config`: initializationOptions and workspace/configuration

- `initialize` sends `initialization_options: self.config.clone()` (`helix-lsp/src/client.rs`).
- For `workspace/configuration`, Helix takes the `config` value itself and walks `item.section.split('.')`. An empty section returns the whole `config`; a missing path returns `null`; `scopeUri` is ignored (`helix-term/src/application.rs`).
- Helix declares dynamic registration for `didChangeConfiguration` and calls it when reloading config.
- That is, Helix uses **one block** for both purposes. This works with servers that use prefixed namespaces (`Lua.*`, `elixirLS.*`, `yaml.*`), so default entries look like `config = { elixirLS.dialyzerEnabled = false }`. Zed and VS Code keep them separate; Lucerna's split is more correct and stays as it is.

### 2.6 `hx --health`

- The top part: `Config file:`, `Language file:`, `Log file:`, the runtime directory, the clipboard provider.
- A table with columns `Language`, `Language servers`, `Debug adapter`, `Formatter`, then one column per kind of tree-sitter query.
- The language server column prints the first server as `✓ name` or `✘ name`, with additional servers on the following lines. The check is only `which(command)` on `$PATH`: it does not spawn the server or run `--version`.
- `hx --health <lang>` prints `Configured language servers:` followed by `✓ name: /resolved/path` or `✘ name: 'cmd' not found in $PATH`, then likewise for the debug adapter, formatter, and query files.
- With no argument it filters by `use-grammars`; `all` / `all-languages` shows everything.

Lucerna already probes with `--version` (`probe_args`), stricter than Helix. The "LSP health" view can follow `hx --health <lang>`: the configured servers, the resolved path or "not found on PATH", with an install hint from the table.

### 2.7 The "Language Server Configurations" wiki

A community page with install instructions and a `languages.toml` snippet for each language. The points used in §7: Phpactor via `phpactor.phar` with `args = ["language-server"]`; intelephense via npm, the premium version needs a `licenceKey`; jdtls via `brew install jdtls`; Elixir via `elixir-ls.zip` and renaming `language_server.sh`; Vue needs `@vue/language-server` + `@vue/typescript-plugin` registered as a plugin of `typescript-language-server`; taplo needs a build with the `lsp` feature (`cargo install taplo-cli --locked --features lsp`, or `brew install taplo`); Gleam uses `gleam lsp` since v0.21; Docker Compose has its own server `docker-compose-langserver --stdio`. The wiki is user-contributed content: treat it only as a hint, not a source of truth.

### 2.8 Similarities and differences with Lucerna today

- Same: servers are a named map; a language keeps an ordered list of servers.
- Different: Helix separates `command` from the language binding and root, while Lucerna/Zed gather everything into `lsp.<server>`. Helix merges `config` as one block, Lucerna splits it in two. Helix has no `!`/`...`.

## 3. License: taking the default table from Helix

Helix is MPL-2.0 (the repo's LICENSE). `languages.toml` has no header of its own (only two comment lines pointing to the docs), so the repo-wide MPL applies. This is not legal advice.

1. **Facts are not copyrightable.** Server names, executable files, mandatory flags (`--stdio`, `serve`, `lsp stdio`), languageIds and the build tool's marker files are facts published by the server projects themselves. US law (Feist v. Rural, 1991) and most countries do not protect facts, only the selection, arrangement and original expression. The part of a ~300-entry TOML file most likely to be protected is the arrangement, the comments, and the Helix-specific fine-tuned `config` values.
2. **MPL-2.0 is file-level copyleft.** If Lucerna copies `languages.toml` (or a large block of it) into, for example, `crates/lucerna-lsp/assets/servers.toml`, that file stays MPL-2.0: its source must be made available under the MPL, notices kept, and modifications to the file are also MPL. MPL §3.3 allows an MPL file inside a "Larger Work" under MIT/Apache, so it is compatible, but:
   - the crate becomes `MIT OR Apache-2.0` plus MPL-2.0, `cargo deny` must allow MPL-2.0, and the `license` field and NOTICE must be changed;
   - it breaks the project's clean "MIT OR Apache-2.0" license story.
3. **Proposed approach:**
   - **Do not copy Helix's file.** Each entry in the default table comes from the server's own upstream documentation (README, docs), which is published fact. Record the upstream URL in that entry's `docs` field; this field is also the install-hint link in the UI.
   - Use Helix only to cross-check "is this command right"; do not copy Helix's file, ordering, comments or `config` blocks.
   - Cross-checkable against nvim-lspconfig (https://github.com/neovim/nvim-lspconfig, Apache-2.0); mason-registry is also Apache-2.0 (unverified). Both are license-compatible. If at some point we derive anything mechanically from either, nvim-lspconfig is the safe source, with an attribution line in NOTICE or in the table header ("server commands cross-checked against nvim-lspconfig, Apache-2.0").
   - A line of thanks ("inspired by Helix's `languages.toml`") in the spec's References section is polite and creates no obligation.
4. Zed's equivalent (`crates/languages`, extensions) is a mix of GPL/Apache; per CLAUDE.md we only learn the design.

## 4. Zed

Local sources: `docs/src/configuring-languages.md`, `assets/settings/default.json`, `crates/settings_content/src/project.rs`.

- `lsp.<server-name>` (`LspSettings`):
  - `binary: { path, arguments, env, ignore_system_version }` (`BinarySettings`): `path` replaces the downloaded binary or the one on PATH, `arguments` replaces the args, `env` adds environment variables.
  - `initialization_options`: JSON merged into the adapter's defaults and then sent in `initialize`.
  - `settings`: JSON returned for `workspace/configuration` (and pushed through `didChangeConfiguration`).
  - `enable_lsp_tasks` (default true), `fetch.pre_release`.
- `languages.<Language>.language_servers`: an ordered list of names; `"!name"` disables a server; `"..."` expands to every other server registered for that language and not yet named (except servers disabled by default unless named). The user's list **fully replaces** the default list (the default `!x` entries are lost unless rewritten). `language_servers: ["..."]` at the top level is the global default.
- `global_lsp_settings`: `button`, `request_timeout` (default 120 s, 0 = no timeout), `max_buffer_line_length` (20000), notifications, semantic token rules.
- **Confirmed: a new server cannot be defined with settings alone.** `LspSettings` has no field that binds a server to a language, a file suffix or a root marker; `language_servers` only orders and filters the names of servers built by an adapter or registered by an extension. An `lsp.foo` entry with an unknown `foo` has no effect. The only route is writing an extension (Rust compiled to WASM, `language_server_command`).

Similarities and differences: Lucerna already has `ServerSettings { binary, initialization_options, settings }`, identical to this shape. The difference to make up is the ability to declare a new server (Helix/Sublime have it, Zed does not).

## 5. VS Code, Sublime LSP, Wu and Zed's terms

### 5.1 VS Code and the generic clients

VS Code core has no generic LSP client; each server comes with an extension built on `vscode-languageclient` (`ServerOptions = { command, args, options: { env, cwd } }`, `LanguageClientOptions = { documentSelector, initializationOptions, synchronize.configurationSection }`). `workspace/configuration` is answered from VS Code's settings by section name (`yaml` gives the `yaml.*` settings).

The generic client extensions (no standard schema):

| Extension | Settings | Shape |
|---|---|---|
| Generic LSP Client "glspc" (llllvvuu/vscode-glspc) | `glspc.serverCommand`, `glspc.serverCommandArguments`, `glspc.languageId`, `glspc.environmentVariables` (expands `$VAR`), `glspc.initializationOptions`, `glspc.trace.server` | one server per workspace |
| vscode-lsp-config (pepebecker) | `lspConfig.servers: [{ language, cmd, args }]` | list |
| Alloglot (MercuryTechnologies) | `alloglot.languages: [{ languageId, serverCommand, formatCommand, onSaveCommand, ... }]`, `alloglot.mergeConfigs` merges `.vscode/alloglot.json` | list, Haskell-leaning |
| langoustine-vscode | `langoustine-vscode.servers` keyed by file suffix | list |

### 5.2 Sublime LSP and Neovim

- Sublime LSP `clients.<name>`: `enabled`, `command` (an argv array), `selector` (scope, for example `source.php`), `env`, `settings`, `initialization_options`, `priority_selector`, `diagnostics_mode`, `disabled_capabilities`, `experimental_capabilities`, `tcp_port`.
- Neovim 0.11+ `vim.lsp.config(name, { cmd, filetypes, root_markers, settings, init_options })` + `vim.lsp.enable(name)` (unverified), with the nvim-lspconfig catalog (Apache-2.0) as the defaults.
- The **common denominator** of Helix, Sublime, Neovim, glspc: `command` + `args` (or argv), a language/file suffix binding, root markers, `env`, `initialization_options`, `settings`. That is Lucerna's minimal schema.

### 5.3 Wu (github.com/dstrml/wu)

Sources: the README, `assets/settings/default.json`, `crates/http_client/src/http_client.rs`, `crates/extension_host/src/extension_host.rs`.

- README: "a fork of Zed" that "diverges heavily from it"; credits the Zed team; described as "The fast, native code editor and agent workspace"; has built-in support for the agents Claude Code, Codex, OpenCode; claims "No account or telemetry".
- License: GPL-3.0-or-later with an Apache-2.0 part (LICENSE-GPL, LICENSE-APACHE). The repo is not a GitHub "fork" object (`fork: false`), created 2026-08-28, last pushed 2026-11-10, about 640 stars.
- Extensions: **still points to Zed's server by default.** `assets/settings/default.json` has `"server_url": "https://zed.dev"` (overridable with the env `ZED_SERVER_URL`); `http_client.rs` keeps `build_zed_api_url`, mapping `https://zed.dev` to `https://api.zed.dev` (and cloud.zed.dev for other endpoints). `extension_host.rs` calls `/extensions/updates` and the other extension endpoints through `build_zed_api_url`. That is, Wu's extension gallery is Zed's registry served from `api.zed.dev`.
- Differences from Zed (per the README, not audited): rebranding (actions namespaced `wu:`), an agent workspace feature, telemetry and accounts removed.
- **Lucerna does not go this way.** The reason is in §5.4.

### 5.4 Zed's terms

Source: https://zed.dev/terms ("Last Updated: March 2, 2026", that is 2026-03-02).

- §2.2 (Access Grant): "Customer agrees to access the Service only through the mechanisms designated by Zed."
- §2.4 (Restrictions), item (b): "(b) seek to access non-public APIs associated with the Service;"

Public documentation for the extension endpoints of `api.zed.dev`: Zed's docs describe the *publishing* workflow for extensions (a submodule PR into `zed-industries/extensions`, packaged by Zed's CI) and the `zed_extension_api` crate for authors (https://zed.dev/docs/extensions/developing-extensions, https://zed.dev/docs/extensions/publishing/prerequisites). **We found no** public documentation for the HTTP endpoints the app uses (`/extensions`, `/extensions/updates`, `/extensions/<id>/download`...); they only show up in Zed's source code. Reading them as "non-public APIs associated with the Service" is the cautious interpretation; a third-party editor calling them is, at best, a gray area and, at worst, a violation of §2.4(b) (a term that binds the person who accepts). This is one reason for Lucerna to use PATH + a table instead of pulling extensions from `api.zed.dev`. The terms are publicly available; what is not public is the API documentation of the endpoints.

## 6. Options for "a language server for every language"

The user approved: **pick f first, d after Phase 4** (if "install and it just works" is needed).

| | Option | Pros | Cons |
|---|---|---|---|
| a | Require `cargo` to build `extension.wasm` from the extension repo | Uses the original Zed extension system | The user must install a Rust toolchain, slow builds, heavy on disk |
| b | Lucerna installs the toolchain itself (rustup, wasm target) | The user has to do nothing | Silently downloads a few hundred MB, many points of failure, hard to support |
| c | Use `api.zed.dev` (like Wu) to fetch prebuilt archives | Easiest, the extension catalog is ready | Conflicts with ToS §2.2 and §2.4(b); depends on Zed's infrastructure |
| d | Lucerna's CI prebuilds `extension.wasm` and hosts it itself (ed25519-signed, license-checked) | Install and it works, independent of Zed, quality and license control | Must operate CI and hosting, deal with supply-chain security; to be Phase 3b after Phase 4 |
| e | Bundle the WASM into the app | Offline, no network needed | Bloated app, updating a server needs an app release, per-extension license issues |
| f | A Helix-style server table (data in the binary, user overrides) | No WASM needed, fast, full LSP features, license-friendly | The user installs servers themselves (with install hints); no custom logic beyond a few hooks (Vue) |
| g | Rust adapters that download servers themselves (like Zed's native adapters) | The best experience for each server | One adapter per server, costly to maintain, must handle downloads and updates |
| h | Ask Zed for permission to use the extension API | Legitimate if agreed | Depends on others, a reply is not certain; still tied to Zed's infrastructure |

## 7. Server table by language

The languageId comes from the "Language identifiers" table of LSP 3.17 (https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#textDocumentItem) when present; ids marked `*` are not in the spec table and are the values VS Code extensions use in practice. "Node" = needs Node.js on PATH; "JVM" = needs a JDK; "BEAM" = needs Erlang/Elixir.

### 7.1 Table

| Language | Server | stdio command | Root markers | languageId | Install on macOS | Needs Node? |
|---|---|---|---|---|---|---|
| PHP | intelephense | `intelephense --stdio` | `composer.json`, `.git` | `php` | `npm i -g intelephense` | Yes |
| PHP | phpactor | `phpactor language-server` | `composer.json`, `.phpactor.json`, `.git` | `php` | `composer global require phpactor/phpactor` or the phar (Homebrew core has no formula) | No (PHP 8.1+) |
| Ruby | ruby-lsp | `ruby-lsp` | `Gemfile`, `.git` | `ruby` | `gem install ruby-lsp` | No (Ruby) |
| Ruby | solargraph | `solargraph stdio` | `Gemfile`, `.git` | `ruby` | `gem install solargraph` | No (Ruby) |
| Java | jdtls | `jdtls` (+ `-data <dir>`) | `pom.xml`, `build.gradle(.kts)`, `settings.gradle(.kts)`, `.git` | `java` | `brew install jdtls` | No (JDK 21+, Python 3) |
| Kotlin | kotlin-lsp (JetBrains) | `kotlin-lsp --stdio` | `settings.gradle(.kts)`, `build.gradle(.kts)`, `pom.xml` | `kotlin`* | `brew install JetBrains/utils/kotlin-lsp` | No (bundled JVM, unverified) |
| Kotlin | kotlin-language-server (fwcd) | `kotlin-language-server` | same as above | `kotlin`* | `brew install kotlin-language-server` (unverified) | No (JVM) |
| Swift | sourcekit-lsp | `sourcekit-lsp` (or `xcrun sourcekit-lsp`) | `Package.swift`, `buildServer.json`, `compile_commands.json`, `.git` | `swift` | ships with Xcode / CLT / the Swift toolchain | No |
| Lua | lua-language-server | `lua-language-server` | `.luarc.json`, `.luarc.jsonc`, `.stylua.toml`, `selene.toml`, `.git` | `lua` | `brew install lua-language-server` | No |
| Elixir | elixir-ls | `elixir-ls` (= `language_server.sh` renamed) | `mix.exs` | `elixir` | `brew install elixir-ls` | No (BEAM) |
| Elixir | expert | `expert --stdio` | `mix.exs` | `elixir` | binary from a GitHub release (`expert_darwin_arm64`) | No (self-contained) |
| Elixir | next-ls | `nextls --stdio` | `mix.exs` | `elixir` | release binary (unverified) | No |
| Elixir | lexical | `lexical` (`start_lexical.sh`) | `mix.exs` | `elixir` | build from source / release | No (BEAM) |
| Zig | zls | `zls` | `build.zig`, `zls.json` | `zig`* | `brew install zls` | No |
| Dart | dart | `dart language-server` | `pubspec.yaml` | `dart` | Dart or Flutter SDK (tap `dart-lang/dart`) | No |
| Vue | vue-language-server | `vue-language-server --stdio` | `package.json` | `vue`* | `npm i -g @vue/language-server` | Yes |
| Vue (TS part) | vtsls + plugin | `vtsls --stdio` | `package.json`, `tsconfig.json` | `vue`* | `npm i -g @vtsls/language-server` | Yes |
| C/C++ | clangd | `clangd` | `compile_commands.json`, `compile_flags.txt`, `.clangd`, `.git` | `c`, `cpp`, `objective-c`, `objective-cpp` | CLT has `clangd`; or `brew install llvm` (keg-only) | No |
| C# | roslyn-language-server | `roslyn-language-server --stdio --autoLoadProjects` | `*.sln`, `*.slnx`, `*.csproj` | `csharp` | `dotnet tool install -g roslyn-language-server --prerelease` | No (.NET SDK, some report needing 10) |
| C# | csharp-ls | `csharp-ls` | `*.sln`, `*.csproj` | `csharp` | `dotnet tool install -g csharp-ls` | No (.NET SDK) |
| C# | OmniSharp | `OmniSharp --languageserver` | `*.sln`, `*.csproj` | `csharp` | GitHub release (the `-net6.0` build) | No (.NET) |
| Haskell | hls | `haskell-language-server-wrapper --lsp` | `hie.yaml`, `cabal.project`, `stack.yaml`, `*.cabal`, `package.yaml` | `haskell` | `ghcup install hls` (or `brew install haskell-language-server`) | No (GHC) |
| OCaml | ocamllsp | `ocamllsp` | `dune-project`, `dune-workspace`, `*.opam`, `esy.json` | `ocaml` | `opam install ocaml-lsp-server` | No (opam switch) |
| Scala | metals | `metals` | `build.sbt`, `build.sc`, `build.mill`, `.scala-build`, `pom.xml`, `build.gradle` | `scala` | `cs install metals` (Coursier: `brew install coursier`) | No (JVM) |
| Terraform | terraform-ls | `terraform-ls serve` | `.terraform`, `.git` | `terraform`* / `terraform-vars`* | `brew install hashicorp/tap/terraform-ls` | No |
| Dockerfile | docker-langserver | `docker-langserver --stdio` | `Dockerfile`, `.git` | `dockerfile` | `npm i -g dockerfile-language-server-nodejs` | Yes |
| Dockerfile | docker-language-server (Docker Inc., Go) | `docker-language-server start --stdio` | same as above | `dockerfile` | GitHub release (unverified) | No |
| YAML | yaml-language-server | `yaml-language-server --stdio` | `.git` | `yaml` | `brew install yaml-language-server` or npm | Yes |
| JSON | vscode-json-language-server | `vscode-json-language-server --stdio` | `.git` | `json`, `jsonc` | `npm i -g vscode-langservers-extracted` | Yes |
| CSS | vscode-css-language-server | `vscode-css-language-server --stdio` | `package.json`, `.git` | `css`, `scss`, `less` | same package | Yes |
| HTML | vscode-html-language-server | `vscode-html-language-server --stdio` | `package.json`, `.git` | `html` | same package | Yes |
| Bash | bash-language-server | `bash-language-server start` | `.git` | `shellscript` | `brew install bash-language-server` or npm | Yes |
| TOML | taplo | `taplo lsp stdio` | `.taplo.toml`, `taplo.toml`, `.git` | `toml`* | `brew install taplo` (the npm build has no LSP) | No |
| Markdown | marksman | `marksman server` | `.marksman.toml`, `.git` | `markdown` | `brew install marksman` | No (self-contained .NET) |
| Nix | nil | `nil` | `flake.nix`, `.git` | `nix`* | `nix profile install nixpkgs#nil` | No (needs Nix) |
| Nix | nixd | `nixd` | `flake.nix`, `.git` | `nix`* | `nix profile install nixpkgs#nixd` | No (needs Nix) |
| Gleam | gleam | `gleam lsp` | `gleam.toml` | `gleam`* | `brew install gleam` | No (BEAM when building) |
| Erlang | elp | `elp server` | `rebar.config`, `.elp.toml`, `erlang.mk` | `erlang` | `brew install erlang-language-platform` | No (Erlang/OTP) |
| Erlang | erlang_ls | `erlang_ls` | `rebar.config`, `erlang_ls.config` | `erlang` | `brew install erlang_ls` (unverified) | No (Erlang/OTP) |

Servers that need Node: intelephense, vue-language-server, vtsls, typescript-language-server, docker-langserver, yaml-language-server, vscode-langservers-extracted (json/css/html), bash-language-server. The Homebrew formulas of yaml-language-server and bash-language-server pull in `node` themselves.

Helix's root markers (for cross-checking, facts only): php `composer.json`, `index.php`; java `pom.xml`, `build.gradle`, `build.gradle.kts`; kotlin `settings.gradle(.kts)`; swift `Package.swift`; elixir `mix.exs`, `mix.lock`; zig `build.zig`; dart `pubspec.yaml`; haskell `Setup.hs`, `stack.yaml`, `cabal.project`, `hie.yaml`; scala `build.sbt`, `build.sc`, `build.gradle(.kts)`, `pom.xml`, `.scala-build`; erlang `rebar.config`; C# `*.slnx`, `*.sln`, `*.csproj`. Ruby and Nix have no `roots`.

### 7.2 Important quirks

**jdtls (Java).**
- `jdtls` on PATH is the Python launcher `bin/jdtls` (`jdtls.py`), which needs **Java 21+** (it checks and exits if missing; `--no-validate-java-version` skips this) and Python 3.
- Workspace data directory: without `-data`, the launcher uses `<cache>/jdtls/jdtls-<sha1 of cwd>` (`~/Library/Caches/jdtls/...` on macOS). So the cwd at spawn matters: spawn with cwd = the LSP root, or pass explicitly `-data <lucerna-cache>/jdtls/<hash-of-root>`. This directory must not be inside the project; one directory per root, not shared by two jdtls processes at once. **Always pass `-data`** inside Lucerna's cache.
- The launcher adds the Equinox flags `-configuration`/OSGi, `-Xms1G`, `--add-modules=ALL-SYSTEM`, `--add-opens` for `java.base`. Extra JVM args go through `--jvm-arg=...`.
- The first run is very slow (importing Maven/Gradle, possibly minutes); it sends `language/status` and `$/progress`. The timeout must be generous.
- The setting `java.import.generatesMetadataFilesAtProjectRoot` (false by default in the current version, unverified) decides whether `.project`/`.classpath`/`.settings` are written into the project.
- jdtls returns `jdt://` URIs (go-to-definition into a library class) that the client must fetch through `java/classFileContents`. The client cannot resolve them yet, so these locations are dropped.

**Vue.**
- `@vue/language-server` 2.x has an optional "hybrid mode"; 3.0 removes the option and only hybrid remains. The Vue server handles the template/style/SFC structure; the TypeScript in `<script>` and template expressions is served by tsserver loading `@vue/typescript-plugin`.
- 3.0 adds a pair of custom notifications: the Vue server sends `tsserver/request` (params `[[id, command, args]]`, exact form unverified) to the client; the client must forward it to the TypeScript server and return `tsserver/response`. If the client ignores it, it logs "Unknown notification" and Vue features silently degrade (lsp-mode issue #4838).
- nvim-lspconfig's `vue_ls` (Apache-2.0) bridges this in `on_init`: it forwards to `vtsls` (or `ts_ls`) through `workspace/executeCommand` with the command `typescript.tsserverRequest`, then notifies `tsserver/response`. vtsls receives the plugin through `settings.vtsls.tsserver.globalPlugins = [{ name = "@vue/typescript-plugin", location = <path of @vue/language-server>, languages = ["vue"], configNamespace = "typescript" }]`.
- Zed does the same in its core: `crates/project/src/lsp_store/vue_language_server_ext.rs` (133 lines) registers a `tsserver/request` handler for the server `vue-language-server`, finds `vtsls` (falling back to `typescript-language-server`) of the same buffer, sends `workspace/executeCommand typescript.tsserverRequest`, and returns `tsserver/response`. Zed's `default.json` sets the Vue servers to `["vue-language-server", "vtsls", "..."]`.
- Helix's defaults only list `vuels` for Vue with `typescript.tsdk = "node_modules/typescript/lib/"`; no handling of `tsserver/request` was found in Helix, so Vue 3.x on Helix only gets the Vue-side features; the wiki tells users to add `typescript-language-server` with the TS plugin.
- A useful minimal setup for a viewer: run `vtsls --stdio` for `.vue` with the global plugin `@vue/typescript-plugin` (location = the global install directory of `@vue/language-server`, found via `npm root -g`), plus `vue-language-server --stdio`, plus a small "request forwarding" hook in Lucerna's LSP client: on receiving `tsserver/request` from server A, call `workspace/executeCommand(typescript.tsserverRequest, ...)` on server B with the same root, then notify A with `tsserver/response`. This is the only Rust piece specific to Vue; the rest is table data.
- Vue 2: the 3.x branch still supports Vue 2.7; the old setup used `@vue/language-server@2` with `vue.hybridMode = false` (takeover), which is deprecated.

**Servers that write into the project directory** (notable for a read-only viewer):

| Server | What it writes | Mitigation |
|---|---|---|
| ruby-lsp | `.ruby-lsp/` (composed bundle), may run bundler (unverified); runs without `bundle exec` | accept or note |
| elixir-ls | `.elixir_ls/`, compiles the project; dialyzer is slow so turn it off (`elixirLS.dialyzerEnabled = false`) | accept |
| expert | `.expert/` (unverified) | accept |
| metals | `.metals/`, `.bloop/`, `project/metals.sbt`; asks to import the build via `window/showMessageRequest` | off by default or ask |
| clangd | `.cache/clangd/index/` | `--background-index=false` (optional) or `.clangd` `Index: Background: Skip` |
| sourcekit-lsp | `.build/index-build/` (background indexing, on by default since Swift 6.1, unverified) | accept |
| jdtls | a data directory outside the project if `-data` is given; optionally `.project`/`.classpath` | always pass `-data` in the cache |
| gleam / zls / hls / ocamllsp | build directories (`build/`, `.zig-cache`, `dist-newstyle`, `_build`) | accept, normal build output |

Suggested fields in the table: `writes_to_project: bool` (shown in the health view), and `enabled_by_default: false` for metals.

**Other quirks (short):**
- intelephense: closed source (free + premium). The premium key goes through `initializationOptions.licenceKey` (British spelling) or `~/intelephense/licence.txt`. `--stdio` is required, otherwise it uses node-ipc. Default: intelephense first, with phpactor as the alternative (Zed sets `["phpactor", "!intelephense", ...]` but the `windows` build reverses it).
- ruby-lsp and solargraph: solargraph needs the `stdio` subcommand. For Ruby, Zed's `["..."]` expands to solargraph before ruby-lsp; Helix puts ruby-lsp first.
- kotlin-lsp: Alpha, "partly closed source"; needs `--stdio` (otherwise it listens on TCP, unverified). Zed prefers `kotlin-lsp` over `kotlin-language-server`; fwcd's is almost unmaintained.
- sourcekit-lsp: on macOS `/usr/bin/sourcekit-lsp` is an xcrun shim, and the toolchain must match the toolchain that builds the project. SwiftPM works right away; Xcode projects need a BSP bridge (`xcode-build-server` writing `buildServer.json`) or `compile_commands.json`.
- lua-language-server: settings use the `Lua.*` namespace; set `Lua.workspace.checkThirdParty = false` to avoid an interactive dialog (unverified).
- elixir-ls and expert: default to `elixir-ls`, with expert as an option; drop next-ls and lexical (replaced, merge history unverified). Expert (https://github.com/elixir-lang/expert) is Alpha and needs `--stdio`.
- zls: must match the Zig compiler version (zls 0.15 for zig 0.15); may run `zig build` on open (unverified).
- dart: `dart language-server` is LSP by default; the README requires the client to identify itself via the env `DASH__IDE_NAME`, `DASH__IDE_VERSION`, `DASH__PLUGIN_NAME`, `DASH__PLUGIN_VERSION`. Useful `initializationOptions`: `onlyAnalyzeProjectsWithOpenFiles` (fits a viewer), `outline`, `flutterOutline`, `suggestFromUnimportedLibraries`, `allowOpenUri`. Flutter has `dart` in `flutter/bin/dart`.
- clangd: without `compile_commands.json`/`compile_flags.txt` it guesses the flags.
- roslyn-language-server: `--autoLoadProjects` to open the solution automatically (otherwise the client must send `solution/open`/`project/open`); only supports pull diagnostics (`textDocument/diagnostic`). Every C# server needs `dotnet restore`. Helix prefers roslyn, omnisharp, csharp-ls; Zed prefers `roslyn`.
- hls: always run the **wrapper** version so it picks the right binary for the project's GHC; slow the first time.
- ocamllsp: must be installed in the same opam switch as the project (often a local `_opam`), so the PATH from a GUI app is often missing it; Lucerna's logic for taking PATH from the login shell matters here.
- terraform-ls: the root is the module directory; `.terraform` only exists after `terraform init`.
- yaml-language-server: schemas via `yaml.schemas`; by default it downloads SchemaStore over the network (`yaml.schemaStore.enable`), so leave the choice to the user.
- JSON/CSS/HTML: the same package `vscode-langservers-extracted` (MIT, split out from VS Code). The JSON server needs the client to push schemas through `json/schemaAssociations` or `json.schemas`, or have the client fetch them via the custom request `vscode/content` (Zed has `json_language_server_ext`); without this it only checks syntax. `provideFormatter: true` turns on formatting.
- bash-language-server: diagnostics from `shellcheck` if it is on PATH; formatting from `shfmt`.
- taplo: the npm package `@taplo/cli` and a few builds lack LSP; use brew or `cargo install taplo-cli --locked --features lsp`. Alternative: `tombi`. Cargo.toml knowledge comes from SchemaStore (network).
- marksman: needs a root (`.marksman.toml` or `.git`) for cross-file links; a single file runs in "single-file mode".
- nil (MIT) or nixd (LGPL-3.0, use the binary only); both need `nix` for evaluation features; formatting through an external `nixfmt`.
- gleam: `gleam lsp` is inside the compiler binary; it builds into `<root>/build/` (unverified).
- Erlang: ELP (`elp server`, configured with `.elp.toml`); Helix runs both erlang_ls and elp, each with `except-features = ["document-symbols", "workspace-symbols"]`; Zed prefers `erlang-ls` and turns `elp` off by default.

## 8. Sketch of one entry in Lucerna's table

For illustration only; keys in snake_case matching the existing settings.

```jsonc
"lsp": {
  "intelephense": {
    "command": "intelephense",          // or binary.path
    "args": ["--stdio"],                // or binary.arguments
    "languages": ["php"],
    "root_markers": ["composer.json", ".git"],
    "language_id": "php",               // default: Lucerna's language id
    "initialization_options": {},
    "settings": {},
    "enabled": true,
    "install": "npm i -g intelephense", // hint in the health view
    "docs": "https://github.com/bmewburn/intelephense-docs"
  }
},
"languages": {
  "PHP": { "language_servers": ["intelephense", "!phpactor", "..."] }
}
```

Proposed merge rule: merge objects by key (like Zed/Helix), replace arrays (like both), `!`/`...` only for `language_servers`. A project-level file may only set `initialization_options`, `settings`, `language_servers` and `enabled` until the directory is trusted.

## 9. What Lucerna chooses

**This is an initial proposal; it will be settled in the spec after discussion with the user.** Points already approved by the user are marked explicitly.

| Topic | Proposal | Why |
|---|---|---|
| Main direction | Option **f**: a Helix-style server table, data in the binary, user overrides (approved) | No WASM or toolchain needed; full LSP; license-friendly |
| Later | Option **d** (Lucerna's CI prebuilds and self-hosts, ed25519-signed) as Phase 3b after Phase 4 if "install and it works" is needed (approved) | Independent of Zed, no ToS conflict |
| Not doing | c (using `api.zed.dev`), and not following Wu | ToS §2.2, §2.4(b); no public documentation for the endpoints |
| Config shape | Keep `lsp.<server>.{binary, initialization_options, settings}` + add `command`/`args`, `languages`, `root_markers`, `language_id`, `enabled`, `install`, `docs` | Compatible with Zed (WASM extensions need exactly this shape), the common denominator of Helix/Sublime/Neovim |
| Server order | `languages.<L>.language_servers` with `!` and `...` | Follows Zed; VS Code has no opinion in its core |
| `config` | Separate `initialization_options` and `settings` | More correct than Helix's single block |
| License of the default table | Write from upstream docs, record `docs`, cross-check nvim-lspconfig, do not copy `languages.toml` | MPL-2.0 is file-level |
| Trust | Built-in and user settings may spawn binaries on PATH; a project file cannot set `command`/`binary.path`/`env` until trusted; consider hashing the project file like Helix | Blocks a malicious branch from changing `command` |
| Health | An "LSP health" view like `hx --health <lang>` but probing `--version` | `probe_args` already exists |
| Custom hooks | Forward `tsserver/request` for Vue; jdtls always gets `-data` in Lucerna's cache | The two places where a data table is not enough |
| Servers that write into the project | A `writes_to_project` flag; metals off by default or asks | A read-only viewer |
| Default server choice | PHP: intelephense; Ruby: ruby-lsp; Kotlin: kotlin-lsp; Elixir: elixir-ls; C#: roslyn; Dockerfile: docker-langserver; Erlang: elp | Following Zed/Helix defaults and maintenance status |

## Sources

- Helix docs: https://docs.helix-editor.com/languages.html
- Helix languages.toml: https://github.com/helix-editor/helix/blob/master/languages.toml
- Helix loader, merge: https://github.com/helix-editor/helix/blob/master/helix-loader/src/lib.rs
- Helix config and trust: https://github.com/helix-editor/helix/blob/master/helix-loader/src/config.rs, https://github.com/helix-editor/helix/blob/master/helix-loader/src/workspace_trust.rs, PR https://github.com/helix-editor/helix/pull/15857
- Helix health: https://github.com/helix-editor/helix/blob/master/helix-term/src/health.rs
- Helix workspace/configuration: https://github.com/helix-editor/helix/blob/master/helix-term/src/application.rs
- Helix initialization options: https://github.com/helix-editor/helix/blob/master/helix-lsp/src/client.rs
- Helix wiki: https://github.com/helix-editor/helix/wiki/Language-Server-Configurations
- Helix release 25.07.1: https://github.com/helix-editor/helix/releases
- MPL-2.0: https://www.mozilla.org/en-US/MPL/2.0/ and FAQ https://www.mozilla.org/en-US/MPL/2.0/FAQ/
- nvim-lspconfig: https://github.com/neovim/nvim-lspconfig (Vue: https://github.com/neovim/nvim-lspconfig/blob/master/lsp/vue_ls.lua)
- LSP language ids: https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#textDocumentItem
- jdtls launcher: https://github.com/eclipse-jdtls/eclipse.jdt.ls/blob/main/org.eclipse.jdt.ls.product/scripts/jdtls.py
- kotlin-lsp: https://github.com/Kotlin/kotlin-lsp
- Expert: https://github.com/elixir-lang/expert
- ELP: https://whatsapp.github.io/erlang-language-platform/docs/get-started/install/
- Dart LSP: https://github.com/dart-lang/sdk/blob/main/pkg/analysis_server/tool/lsp_spec/README.md
- ruby-lsp: https://shopify.github.io/ruby-lsp/
- roslyn-language-server: https://www.nuget.org/packages/roslyn-language-server.linux-x64, https://github.com/seblyng/roslyn.nvim, https://zed.dev/docs/languages/csharp.html
- Vue language tools: https://github.com/vuejs/language-tools, wiki https://github.com/vuejs/language-tools/wiki/Neovim, README https://cdn.jsdelivr.net/npm/@vue/language-server@3.3.1/README.md, PR https://github.com/vuejs/language-tools/pull/5248, lsp-mode issue https://github.com/emacs-lsp/lsp-mode/issues/4838
- Wu: https://github.com/dstrml/wu
- Zed ToS: https://zed.dev/terms
- Zed extension docs: https://zed.dev/docs/extensions/developing-extensions, https://zed.dev/docs/extensions/publishing/prerequisites
- Zed (local): `docs/src/configuring-languages.md`, `assets/settings/default.json`, `crates/settings_content/src/project.rs`, `crates/project/src/lsp_store/vue_language_server_ext.rs`
- VS Code LSP guide: https://code.visualstudio.com/api/language-extensions/language-server-extension-guide
- glspc: https://github.com/llllvvuu/vscode-glspc
- vscode-lsp-config: https://github.com/pepebecker/vscode-lsp-config
- Alloglot: https://github.com/MercuryTechnologies/alloglot
- langoustine-vscode: https://github.com/neandertech/langoustine-vscode
- Sublime LSP: https://lsp.sublimetext.io/client_configuration/
