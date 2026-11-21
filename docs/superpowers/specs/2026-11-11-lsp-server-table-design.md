# Language server table (Helix-style)

Date 2026-11-11. Status: implemented (commits 92d55bd–0c85e21, plus the docs commit that fixes this line; includes the fixes after the final review). Differences from the spec at implementation time: clangd's `languages` is `["c","c++","objective-c"]` because extension ids are lowercase names; clangd's root markers stay as in Phase 3 (`compile_commands.json`, `.clangd`), without adding `compile_flags.txt` or `.git`; the clangd install command depends on the OS (`xcode-select --install` on macOS, `sudo apt install clangd` on Linux); the relay waits for a vtsls that is `Starting` instead of returning `null` right away; Health's `probe` command runs in the root directory when the directory is trusted, and in `$HOME` when it is not; Health's summary row counts servers (each server once), not (language, server) pairs; phpactor is installed with `composer global require phpactor/phpactor` because Homebrew core has no formula for it.

## 1. Goals

PHP, Ruby, Java, Kotlin, Swift, Lua, Elixir, Zig, Dart, Vue, C#, Haskell… get LSP in Lucerna when the user **installs the server on PATH themselves**, with no WASM host and nothing for Lucerna to download. The approach follows Helix: a default server table lives in the core, and users override or add servers through settings JSON.

Settled with the user:

| # | Decision | Reason |
|---|---|---|
| T1 | **Extend the existing `lsp` key** instead of adding a new key. The default table and the user's settings share one shape. | One place to configure a server; stays compatible with the existing Zed-style keys (`binary`, `initialization_options`, `settings`). |
| T2 | **Match by file extension/name too** (`file_types`), not only by language id. | Ruby, PHP… files still get LSP when no language extension is installed (only highlighting is missing). Helix also declares file-types in its table. |
| T3 | **Do Vue fully, with a relay** of `tsserver/request` between vue-language-server and vtsls. | Without the relay, the `<script>` part gets no hover or go-to-definition. Zed does the relay in its core; Helix does not. |
| T4 | **⚡ popover + Health tab + `luc lsp health`.** | The user knows which server is missing and the install command, like `hx --health`. |
| T5 | **The table is JSON data** (`servers.json`), one `ServerDef` struct in place of the `Adapter` trait and 6 Rust adapters. Special behavior is switched on by data (the `{data_dir}` variable, `"relay"`). | One code path; the table is easy to read and to source line by line; users override it with exactly that shape. |
| T6 | **Do not copy Helix's `languages.toml`** (MPL-2.0, file-level copyleft). Each line is written from the server's own upstream docs, with `homepage` recorded; it can be cross-checked against nvim-lspconfig (Apache-2.0). | Commands, arguments and root markers are facts; this avoids putting an MPL file in the repo. |
| T7 | **Amend Phase 3's D3** (see §8). | There is no WASM host yet, and this is the cheapest way to get LSP for every language; the risk of running arbitrary commands is already blocked by the directory trust mechanism. |

**Success when:**
- Install `intelephense` (npm), then open a `.php` file in a trusted directory, even without the PHP extension installed: diagnostics, hover, F12 and `@` work.
- Open a `.vue` file with `@vue/language-server` and `@vtsls/language-server` installed: hovering in `<script setup>` shows TypeScript types.
- With a server missing: the ⚡ popover says "not found on PATH" with the install command and a Copy button; the Health tab and `luc lsp health` list every language in the table.
- A user who adds a server the table does not have (for example `"lsp": { "mylang-ls": { "command": "mylang-ls", "file_types": ["ml"] } }`) gets it running without rebuilding Lucerna.
- All Phase 3 tests stay green.

**Out of scope:** downloading servers automatically; Helix's `only-features` / `except-features`; the WASM host (3b, per option d after Phase 4); a UI for editing the table (users edit JSON); multi-root workspaces.

## 2. References

Details are in `docs/research/2026-11-11-lsp-server-table.md`. Summary:
- **Helix:** `[language-server.<name>]` (`command`, `args`, `config`, `timeout`, `environment`, `required-root-patterns`) and `[[language]]` (`language-servers`, `roots`, `language-id`, `file-types`). Three layers merge (default → user → `.helix/languages.toml`): tables merge by key, arrays are replaced whole. `config` is sent as `initializationOptions` and used to answer `workspace/configuration`. `hx --health` only runs `which` and does not start the server.
- **Zed:** `lsp.<name>.{binary{path,arguments,env}, initialization_options, settings}` and `languages.<L>.language_servers` with `!`/`...`; a new server **cannot** be defined by settings alone (every new server must come from an extension). The Vue relay lives in the core (`vue_language_server_ext.rs`).
- **VS Code:** no generic LSP client in the core; each extension configures itself. The "generic client" extensions (glspc…) use their own keys, with no common standard.
- **Wu** (`dstrml/wu`, a GPL fork of Zed) still downloads extensions from `api.zed.dev`. Lucerna does not go this way: Zed's terms (§2.2, §2.4(b)) only allow access through the mechanism Zed designates and forbid non-public APIs.
- **Lucerna departs from Zed and VS Code** by allowing new servers to be defined in settings (§8, following Helix).

## 3. Data model

### 3.1 A server entry

`lsp.<name>` has these fields (new fields marked *):

| Field | Type | Meaning |
|---|---|---|
| `command`* | string | Binary name looked up on the login shell's PATH, or an absolute path |
| `args`* | array of strings | Arguments; variables are supported (§3.3) |
| `env`* | object string → string | Extra environment variables (merged onto the login shell environment) |
| `languages`* | array of Lucerna language ids | For example `"php"`, `"vue.js"`, `"c++"` (extension id = lowercase name, spaces become `-`) |
| `file_types`* | array | File extension (`"rb"`) or full file name (`"Gemfile"`); matching is case-insensitive for extensions and case-sensitive for file names |
| `language_id`* | string, or object Lucerna id → LSP id | `languageId` in `didOpen`. If absent, the Lucerna language id is used; a file with no language uses the string value, or the first element of `languages` |
| `root_markers`* | array | File/directory names that mark the root; globs such as `*.sln`, `*.cabal` are supported |
| `root_search`* | `"nearest"` \| `"outermost"` \| `"ordered"` | Same as the existing `RootSearch` (`ordered` = `NearestByMarker`) |
| `probe`* | array of strings or `false` | Command run as a trial when looking for the binary (default `["--version"]`) |
| `enabled`* | bool | Default `true`; alternative servers in the default table are `false` |
| `relay`* | `"vue-tsserver"` | Turns on the Vue relay (§5) |
| `install`* | string, or object `macos`/`linux`/`windows` → string | Install command shown in Health |
| `homepage`* | URL | Upstream docs; also the source of this line |
| `note`* | string | One-line note shown in the Health tab (for example the server writes into the project directory) |
| `binary` | object `{path, arguments, env}` | Already exists (Zed-style); `path` replaces `command`, `arguments` replaces `args`, `env` merges onto `env` |
| `initialization_options` | object | Already exists |
| `settings` | object | Already exists (answers `workspace/configuration`, `didChangeConfiguration`) |

An entry is **usable** when it has `command` (or `binary.path`) and at least one of `languages` / `file_types`. An entry that only overrides a few fields of a default server (for example only `settings`) is still valid because it is merged with the default line.

### 3.2 Layer merging

- Three layers: **default** (`crates/lucerna-lsp/assets/servers.json`, embedded with `include_str!`) → **User** → **Workspace**. The User and Workspace layers are the `lsp` value that `SettingsStore` has already merged (objects merge recursively, the upper layer wins; non-object values replace whole).
- Merging the default with settings follows the same rules: objects merge by key recursively; arrays (`args`, `languages`, `file_types`, `root_markers`, `probe`) are replaced whole.
- **Trust:** when the directory is untrusted, Lucerna runs no server and does not read the Workspace layer's `lsp` (as in Phase 3, `get_trusted`). When trusted, the Workspace layer is read in full, including `command`.
- Choosing servers for a language still uses `lucerna.lsp.languageServers` to override per language (`"[php]": { … }`), with `!name` and `"..."`. `"..."` covers every **enabled** server that matches the file (§4.1), in name order.
- The merge result is `ServerTable { servers: BTreeMap<Arc<str>, ServerDef>, errors: Vec<TableError> }`. An entry of the wrong type or missing a required field goes into `errors` (with the key name and the reason) and is skipped; the other entries stay usable.

### 3.3 Variables in `args` and `env`

| Variable | Value |
|---|---|
| `{root}` | The server's root (§4.2) |
| `{data_dir}` | `<Lucerna data dir>/lsp/<server name>/<short hash of root>`; created if missing |
| `{npm_root}` | Result of `npm root -g` (run in the background once per process, 5 s timeout, using the login shell environment) |

An unknown variable, or a failed `{npm_root}`, moves that server to `Failed` with a message naming the variable. No variable is allowed in `command`.

## 4. File matching and lifecycle

### 4.1 Choosing servers for a file

1. If the file has a Lucerna language (id `L`) and the table has servers (enabled or not) that list `L` in `languages`: those servers.
2. If there is no language, or no server lists `L` (an extension named differently from the table, for example "Terraform Vars" → `terraform-vars` for `.tfvars`, "OCaml Interface" → `ocaml-interface` for `.mli`): the servers whose `file_types` contain the file's extension or name.
3. Filter by `L`'s `lucerna.lsp.languageServers` (or the default `["..."]` when there is no language) and by `enabled`.
4. `languageId` in `didOpen` follows `language_id` (§3.1).

`Documents` emits `DocumentEvent::Opened` even when `language_id` is `None`; `LanguageServers` uses the path to match `file_types`. The "file too large" and "untrusted" checks stay unchanged.

### 4.2 Root, startup, multiple servers

- The root is found by `root_markers` and `root_search` as in Phase 3; a marker with `*` or `?` is a glob over the entry names in each parent directory. A file outside the workspace uses the running server (as in the final Phase 3 fix).
- `ServerKey.server` changes from `&'static str` to `Arc<str>` because the server name now comes from data.
- A file can be attached to several servers (already the case). Every request (hover, definition, symbols…) merges as in Phase 3.
- When settings change: recompute the table; a server whose `command`/`args`/`env`/`binary`/`initialization_options` changed is restarted, one whose only `settings` changed gets `didChangeConfiguration`, one removed or disabled is stopped (the existing `SettingsDiff` logic, comparing on `ServerDef`).

## 5. Vue

- Default table: `vue-language-server` (`--stdio`, `languages: ["vue.js"]`, `file_types: ["vue"]`, `language_id: "vue"`, `relay: "vue-tsserver"`) and `vtsls` (`--stdio`, `languages: ["vue.js"]`, `file_types: ["vue"]`, `language_id: "vue"`), with vtsls's `settings` (answering `workspace/configuration`, as in nvim-lspconfig) declaring the plugin:

  ```json
  { "vtsls": { "tsserver": { "globalPlugins": [ {
      "name": "@vue/typescript-plugin",
      "location": "{npm_root}/@vue/language-server",
      "languages": ["vue"], "configNamespace": "typescript", "enableForWorkspaceTypeScriptVersions": true
  } ] } } }
  ```

  (Variables are also substituted in `settings` and `initialization_options`.)
- **Relay** (in `lucerna-lsp`, enabled by `relay: "vue-tsserver"`): vue-language-server sends a `tsserver/request` notification with an array of `[id, command, args]`. For each element, Lucerna finds a `Running` `vtsls` with the same root (or else `typescript-language-server`, as Zed does), sends `workspace/executeCommand { command: "typescript.tsserverRequest", arguments: [command, args, { isAsync: true, lowPriority: true }] }`, then sends vue-language-server a `tsserver/response` notification with `[[id, body]]` (body is the reply's `result.body`, `null` on error). If there is no TypeScript server with the same root, it immediately replies `[[id, null]]` so vue-language-server does not hang; if that server is `Starting`, the request waits until it is `Running` (then sends) or fails/stops (then replies `null`), because vue-language-server sends `_vue:projectInfo` right after `didOpen`, when vtsls usually has not finished starting (nvim-lspconfig retries 10 times × 100 ms for the same reason). Each request runs in the background and is answered separately, without blocking other events.
- **Verified shape** (2026-11-11) against `vuejs/language-tools` (MIT): `packages/language-server/lib/server.ts` sends `sendNotification('tsserver/request', [id, command, args])` and receives `tsserver/response` with `([id, res]) => …`; `vscode-jsonrpc` wraps a single array parameter in a params array, so on the wire it is `[[id, command, args]]` and `[[id, body]]`. The VS Code client (`extensions/vscode/src/extension.ts`) calls `typescript.tsserverRequest` with `command, args, { isAsync: true, lowPriority: true }` and returns `res?.body`. nvim-lspconfig's `lsp/vue_ls.lua` (Apache-2.0) also unwraps two array levels and returns `{ { id, r and r.body } }`, but drops the third object in `arguments`. Lucerna follows the VS Code client; this matches the description above.
- Plain TypeScript/JavaScript still uses `typescript-language-server` (as in Phase 3). vtsls is attached only to `.vue`; a user who wants vtsls for TS adds `"typescript"` to its `languages` and selects it with `languageServers`.

## 6. jdtls

Table line: `"command": "jdtls", "args": ["-data", "{data_dir}"]` (Homebrew's launcher script, needs JDK 21+). `{data_dir}` lives in Lucerna's data dir so jdtls does not write workspace data into the project. No dedicated Rust adapter is needed.

## 7. Health

- **⚡ popover:** a `NotFound` server row for the open files additionally shows the install command for the current OS (code font, muted) and a text button "Copy" (copies the command to the clipboard). Table errors (`TableError`) appear as their own row "Invalid `lsp.<name>`: <reason>". The bottom of the popover gets a "Show All Languages…" button that opens the Health tab. Reuses `popover`, `text_button`, `tooltip` from `ui.rs`.
- **Health tab** (`lucerna.lsp.health`, `TabItem::LspHealth`, a single tab, not saved in workspace state): a read-only list, one row per (language, server) pair in the table, grouped by language with headers like `excerpt_list`. Columns: server, status (`✓ /path/to/binary` in `text.muted`, `✗ not found` in `warning`, `disabled` muted), install command + Copy button. The top of the tab has a summary row "18 of 40 servers found" (counts servers, each once) and a refresh button; table errors appear right below, in the same row form as the popover. `which` and `probe` run in the background, and the results are cached until refresh is clicked, settings change, or the directory's trust state changes. `probe` runs in the root when the directory is trusted, otherwise in `$HOME` (rustup, asdf and mise shims read the directory's config files).
- **`luc lsp health`:** prints the same table to stdout as columns (language, server, status, install command), with no GUI needed; exit code 0. Uses `lucerna-lsp` directly and the User settings (does not read the current directory's Workspace settings).
- Servers that write into the project directory (ruby-lsp `.ruby-lsp/`, elixir-ls `.elixir_ls/`, metals `.metals/` `.bloop/`, clangd `.cache/clangd/`, sourcekit-lsp `.build/`) get a one-line note in the Health tab. Not blocking: the directory has already passed the trust step.

## 8. Changes to older specs

- **Phase 3, D3** changes from "Do not allow declaring a server by settings alone" to: "**Allow declaring a server by settings** (`lsp.<name>.command`…), following Helix. Departs from VS Code and Zed. Reason: there is no WASM host yet, and this is the cheapest way to get LSP for every language; the risk of running arbitrary commands is already blocked by the directory trust mechanism (Workspace settings are only read when trusted)." Phase 3's "Out of scope" section drops "user-declared servers".
- Phase 3 §3.1 `adapters`: the table of Rust adapters is replaced by `servers.json`; the values of the rust-analyzer, gopls, basedpyright/pyright, typescript-language-server and clangd lines stay unchanged.
- Phase 3 §8 settings: `lsp` gains the §3.1 fields.

## 9. Default table

Each language has exactly one server enabled by default; alternative servers are in the table with `enabled: false`. Commands and root markers come from upstream docs (the "Source" column = `homepage`), cross-checked against nvim-lspconfig. The "Language" column is the Lucerna id (bundled or from a common extension); `file_types` lists the matching extensions.

| Language (id) | Enabled server | Command | Root markers | Install (macOS) | Disabled by default |
|---|---|---|---|---|---|
| rust | rust-analyzer | `rust-analyzer` | `Cargo.toml` (outermost) | `rustup component add rust-analyzer` | |
| go | gopls | `gopls` | `go.work`, `go.mod` (ordered) | `go install golang.org/x/tools/gopls@latest` | |
| python | basedpyright | `basedpyright-langserver --stdio` | `pyproject.toml`, `setup.py`, `setup.cfg`, `pyrightconfig.json` | `npm i -g basedpyright` | pyright |
| typescript, typescriptreact, javascript, javascriptreact | typescript-language-server | `… --stdio` | `tsconfig.json`, `jsconfig.json`, `package.json` | `npm i -g typescript typescript-language-server` | |
| php | intelephense | `intelephense --stdio` | `composer.json`, `.git` | `npm i -g intelephense` | phpactor |
| ruby | ruby-lsp | `ruby-lsp` | `Gemfile`, `.git` | `gem install ruby-lsp` | solargraph |
| java | jdtls | `jdtls -data {data_dir}` | `pom.xml`, `build.gradle`, `build.gradle.kts`, `settings.gradle`, `settings.gradle.kts`, `.git` | `brew install jdtls` | |
| kotlin | kotlin-lsp | `kotlin-lsp --stdio` | `settings.gradle.kts`, `settings.gradle`, `build.gradle.kts`, `build.gradle`, `pom.xml` | `brew install JetBrains/utils/kotlin-lsp` | kotlin-language-server |
| swift | sourcekit-lsp | `sourcekit-lsp` | `Package.swift`, `buildServer.json`, `compile_commands.json`, `.git` | Xcode / Command Line Tools | |
| lua | lua-language-server | `lua-language-server` | `.luarc.json`, `.luarc.jsonc`, `.stylua.toml`, `selene.toml`, `.git` | `brew install lua-language-server` | |
| elixir | elixir-ls | `elixir-ls` | `mix.exs` | `brew install elixir-ls` | expert |
| zig | zls | `zls` | `build.zig`, `zls.json` | `brew install zls` | |
| dart | dart | `dart language-server --protocol=lsp` | `pubspec.yaml` | Dart SDK or Flutter | |
| vue.js | vue-language-server + vtsls | §5 | `package.json` | `npm i -g @vue/language-server @vtsls/language-server` | |
| c, c++, objective-c | clangd | `clangd` | `compile_commands.json`, `.clangd` | `xcode-select --install` (Linux: `sudo apt install clangd`) | |
| c# | csharp-ls | `csharp-ls` | `*.sln`, `*.slnx`, `*.csproj` | `dotnet tool install -g csharp-ls` | roslyn-language-server |
| haskell | haskell-language-server | `haskell-language-server-wrapper --lsp` | `hie.yaml`, `cabal.project`, `stack.yaml`, `*.cabal`, `package.yaml` | `ghcup install hls` | |
| ocaml | ocamllsp | `ocamllsp` | `dune-project`, `dune-workspace`, `*.opam` | `opam install ocaml-lsp-server` | |
| scala | metals | `metals` | `build.sbt`, `build.sc`, `build.mill`, `.scala-build` | `cs install metals` | |
| terraform | terraform-ls | `terraform-ls serve` | `.terraform`, `.git` | `brew install hashicorp/tap/terraform-ls` | |
| dockerfile | docker-langserver | `docker-langserver --stdio` | `Dockerfile`, `.git` | `npm i -g dockerfile-language-server-nodejs` | |
| yaml | yaml-language-server | `yaml-language-server --stdio` | `.git` | `brew install yaml-language-server` | |
| json, jsonc | vscode-json-language-server | `… --stdio` | `.git` | `npm i -g vscode-langservers-extracted` | |
| css, scss, less | vscode-css-language-server | `… --stdio` | `package.json`, `.git` | as above | |
| html | vscode-html-language-server | `… --stdio` | `package.json`, `.git` | as above | |
| shellscript | bash-language-server | `bash-language-server start` | `.git` | `brew install bash-language-server` | |
| toml | taplo | `taplo lsp stdio` | `.taplo.toml`, `taplo.toml`, `.git` | `brew install taplo` | |
| markdown | marksman | `marksman server` | `.marksman.toml`, `.git` | `brew install marksman` | |
| nix | nil | `nil` | `flake.nix`, `.git` | `nix profile install nixpkgs#nil` | nixd |
| gleam | gleam | `gleam lsp` | `gleam.toml` | `brew install gleam` | |
| erlang | elp | `elp server` | `rebar.config`, `.elp.toml`, `erlang.mk` | `brew install erlang-language-platform` | |

A test checks that Lucerna's bundled language ids (`shellscript`, `json`…) are in the registry and named by the table; extension ids (`php`, `ruby`, `vue.js`, `c++`, `c#`…) are only checked for the right form (lowercase, no spaces), not for matching a real extension, because extensions are not available during tests. A file whose id no server lists still gets a server through `file_types` (§4.1, covered by a test).

## 10. Code architecture

- `crates/lucerna-lsp/assets/servers.json`: the default table.
- `crates/lucerna-lsp/src/table.rs` (new): `ServerDef`, `ServerTable::build(defaults: &Value, user_and_workspace: &Value) -> ServerTable`, layer merging, per-entry validation, `TableError`.
- `crates/lucerna-lsp/src/vars.rs` (new): substitutes `{root}`, `{data_dir}`, `{npm_root}`.
- `crates/lucerna-lsp/src/relay.rs` (new): the Vue relay.
- `crates/lucerna-lsp/src/adapters.rs`: drop the `Adapter` trait and the `Static`s; keep `find_root` (taking `&ServerDef`) and add glob.
- `crates/lucerna-lsp/src/settings.rs`, `registry.rs`: use `ServerDef`, `ServerKey.server: Arc<str>`, `select_servers` per §4.1.
- `crates/lucerna-lsp/src/health.rs` (new): `health(table, launcher) -> Vec<HealthRow>`, shared by the tab and the CLI.
- `crates/lucerna-workbench`: `language_servers` uses `ServerTable` and matches `file_types`; `lsp_status.rs` adds the install command + Copy + table errors; `lsp_health.rs` (new) for the Health tab; `TabItem::LspHealth`.
- `crates/lucerna/src/`: the `luc lsp health` subcommand (like `luc ext`).

## 11. Error handling

- Bad entry: skip that entry, report it in the ⚡ popover and the Health tab; no panic, no effect on other servers.
- `npm root -g` fails or times out: only servers using `{npm_root}` move to `Failed`, with the message "npm not found; {npm_root} unavailable".
- The relay cannot find vtsls, or vtsls returns an error: reply `[[id, null]]`.
- A broken `servers.json` is a build-time error (caught by a test), not a runtime error.

## 12. Testing

- **Default table:** parses; every line has `command`, has `languages` or `file_types`, has `install` and `homepage`; every language has exactly one `enabled` server.
- **`table.rs`:** merging (objects merge, arrays replace, User beats default, Workspace beats User); a bad entry goes into `errors`; overriding only `settings` of a default server.
- **Matching:** by language; by extension and file name when there is no language; `language_id` as string and as object; `languageServers` with `!`/`...`.
- **Variables:** `{root}`, `{data_dir}` (creates the directory), `{npm_root}` (fake launcher), unknown variable.
- **Root:** glob `*.sln`.
- **Relay:** two `FakeServer`s (vue and vtsls): `tsserver/request` → `workspace/executeCommand` to vtsls → `tsserver/response` with the right id; with no vtsls, replies `null`.
- **Workbench:** opening `.rb` with no Ruby language still starts the Ruby server (fake); the popover shows the install command and Copy copies the exact string; the Health tab lists all rows and statuses; a server declared in User settings runs; one declared in `.lucerna/settings.json` runs only when trusted.
- **CLI:** snapshot of `luc lsp health` with a fake launcher.
- **Phase 3:** all old tests green.
- **Real runs:** rust-analyzer, clangd (and npm/brew servers if installed) on the development machine.
