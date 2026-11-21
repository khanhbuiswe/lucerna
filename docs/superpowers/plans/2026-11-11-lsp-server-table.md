# Language Server Table Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace Lucerna's six Rust LSP adapters with a Helix-style data table (`servers.json`) that users extend through the `lsp` setting, add file-type matching, the Vue `tsserver/request` relay, `{data_dir}` for jdtls, install hints in the ⚡ popover, a Health tab and `luc lsp health`.

**Architecture:** `lucerna-lsp` gains `table.rs` (`ServerDef`, `ServerTable` built by merging the embedded defaults with the `lsp` setting), `vars.rs` (argument variables), `relay.rs` (Vue relay helpers) and `health.rs`; the `Adapter` trait goes away and every caller takes `&ServerDef`. `lucerna-workbench` builds the table from settings, matches documents by language or file type, routes the relay, and shows install hints and a Health tab. `lucerna` adds `luc lsp health`.

**Tech Stack:** Rust 2024, serde/serde_json, globset (glob root markers), sha2 (data-dir hash), gpui (workbench only), clap (CLI), insta (CLI snapshot).

**Spec:** `docs/superpowers/specs/2026-11-11-lsp-server-table-design.md` (`§n` below). Research: `docs/research/2026-11-11-lsp-server-table.md`. Builds on Phase 3: `docs/superpowers/specs/2026-11-10-lucerna-phase3-lsp-design.md`.

## Global Constraints

- Only `lucerna-workbench` depends on gpui; `lucerna-lsp` stays gpui- and tokio-free.
- No UI-thread blocking: `which`, probes, `npm root -g`, root search and Health checks run off the UI thread; panics contained with `tasks::guarded` / `catch_panic`.
- Never copy Helix `languages.toml` (MPL-2.0) or Zed GPL code. Every `servers.json` entry is written from the server's upstream docs and carries `homepage`.
- Untrusted folder: no server runs and the Workspace-layer `lsp` value is never read (unchanged from Phase 3).
- Merge rule: objects merge by key recursively, upper layer wins; arrays and scalars are replaced whole (§3.2).
- Variables: `{root}`, `{data_dir}` = `<data dir>/lsp/<server>/<first 12 hex of sha256(root path)>`, `{npm_root}` = `npm root -g` (5 s timeout, login-shell env, once per process). Unknown variable → server `Failed` (§3.3).
- Exact copy: `"not found on PATH"` row keeps the Phase 3 message; table error row `Invalid \`lsp.<name>\`: <reason>`; popover button `Copy`; popover footer button `Show All Languages…`; Health summary `"{found} of {total} servers found"`; npm failure `npm not found; {npm_root} unavailable`.
- Health tab: `TabItem::LspHealth`, singleton, never persisted. Command `lucerna.lsp.health`. CLI `luc lsp health`, exit code 0, reads User settings only.
- Phase 3 tests stay green; behaviour for rust-analyzer / gopls / basedpyright / pyright / typescript-language-server / clangd is unchanged (same commands, args, roots, probes).
- Cargo: `export PATH="/opt/homebrew/opt/rustup/bin:$PATH"`; never `--release` during development; keep ≥ 1.2 GB free. Commit on `main`, stage explicit paths, trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. **A user override touches only `settings` of a default server** → the default `command`/`args`/roots stay, only `settings` changes, no restart (Task 1 `override_settings_only_keeps_default_launch`, Task 3 `settings_only_override_reconfigures`).
2. **A user entry is malformed (`"args": "--stdio"` string instead of array, or no `command` and no default to merge with)** → that entry is skipped with a `TableError`, every other server still runs (Task 1 `bad_entry_is_reported_and_skipped`, Task 6 popover row test).
3. **A file with no Lucerna language but a matching `file_types` entry, opened in a split next to a file that has the language** → both get the same server process (one root key), `languageId` correct for each (Task 3 `file_type_and_language_share_one_server`).
4. **`.lucerna/settings.json` in an untrusted repo defines `lsp.evil.command`** → nothing launched, Health does not list it as found; after trust it runs (Task 3 `workspace_defined_server_runs_only_when_trusted`).
5. **Vue opened before vtsls finished starting / vtsls missing** → `tsserver/response` with `null` body, vue-language-server keeps working, no hang (Task 5 `relay_without_typescript_server_answers_null`).

---

## File map

- `crates/lucerna-lsp/assets/servers.json` — default table (§9).
- `crates/lucerna-lsp/src/table.rs` (new) — `ServerDef`, `ServerTable`, merge, validation.
- `crates/lucerna-lsp/src/vars.rs` (new) — variable expansion, `data_dir_for`, `npm_root`.
- `crates/lucerna-lsp/src/relay.rs` (new) — parse `tsserver/request`, build `tsserver/response`.
- `crates/lucerna-lsp/src/health.rs` (new) — `HealthRow`, `health()`, `format_table()`.
- `crates/lucerna-lsp/src/adapters.rs` — remove `Adapter`, `Static`, `builtin_adapters`; keep `RootSearch`, `find_root(&ServerDef, …)` with glob markers.
- `crates/lucerna-lsp/src/settings.rs` — `select_servers`/`resolve_launch` over `ServerDef`; `BinarySettings` stays; `ServerSettings`/`ServersSettings` removed (folded into `ServerDef`).
- `crates/lucerna-lsp/src/registry.rs` — `ServerKey.server: Arc<str>`; `Registry` holds a `ServerTable`.
- `crates/lucerna-lsp/src/launcher.rs` — `Launcher::output`.
- `crates/lucerna-lsp/src/client.rs` — `ServerEvent::Notification`.
- `crates/lucerna-lsp/src/test_support.rs` — `FakeLauncher::with_output`.
- `crates/lucerna-workbench/src/language_servers/{mod.rs,lifecycle.rs,sync.rs,pump.rs}` — table, file types, relay routing.
- `crates/lucerna-workbench/src/lsp_status.rs` — install line, Copy, table errors, footer button.
- `crates/lucerna-workbench/src/lsp_health.rs` (new) — Health tab view.
- `crates/lucerna-workbench/src/editor_groups/{model.rs,view.rs}` — `TabItem::LspHealth`.
- `crates/lucerna/src/{main.rs,lsp_cli.rs (new)}` — `luc lsp health`.
- Docs: Phase 3 spec D3, §3.1, §8; this feature's spec status; `THIRD_PARTY_NOTICES.md` only if a source requires it (MIT/Apache docs facts do not).

---

## Task 1: `ServerDef`, `ServerTable`, merge and the six existing servers as data

**Files:** Create `crates/lucerna-lsp/src/table.rs`, `crates/lucerna-lsp/assets/servers.json`; modify `crates/lucerna-lsp/src/lib.rs`, `crates/lucerna-lsp/src/adapters.rs` (serde on `RootSearch`); test `crates/lucerna-lsp/tests/table.rs`.

**Interfaces — Produces:**
```rust
#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)] #[serde(rename_all = "lowercase")]
pub enum RootSearch { #[default] Nearest, Outermost, #[serde(rename = "ordered")] NearestByMarker }   // adapters.rs, existing enum
#[derive(Deserialize, Clone, Debug, PartialEq)] #[serde(untagged)]
pub enum LanguageIdMap { One(String), Map(BTreeMap<String, String>) }
#[derive(Deserialize, Clone, Debug, PartialEq)] #[serde(untagged)]
pub enum Install { One(String), PerOs(BTreeMap<String, String>) }   // keys "macos" | "linux" | "windows"
#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)] #[serde(rename_all = "kebab-case")]
pub enum Relay { VueTsserver }
#[derive(Clone, Debug, PartialEq)]
pub struct ServerDef {
    pub name: Arc<str>,                       // the `lsp` key
    pub command: Option<String>,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub languages: Vec<String>,
    pub file_types: Vec<String>,
    pub language_id: Option<LanguageIdMap>,
    pub root_markers: Vec<String>,
    pub root_search: RootSearch,
    pub probe: Option<Vec<String>>,           // None when `"probe": false`; default ["--version"]
    pub enabled: bool,                        // default true
    pub relay: Option<Relay>,
    pub install: Option<Install>,
    pub homepage: Option<String>,
    pub note: Option<String>,                 // one-line Health note (e.g. writes into the project)
    pub binary: Option<BinarySettings>,       // existing struct in settings.rs
    pub initialization_options: Option<Value>,
    pub settings: Option<Value>,
}
impl ServerDef {
    pub fn lsp_language_id(&self, lucerna_language: Option<&str>) -> String;  // §3.1 rule
    pub fn matches(&self, language: Option<&str>, path: &Path) -> bool;       // §4.1 steps 1–2
    pub fn install_for(&self, os: &str) -> Option<&str>;
    pub fn launch_fields_eq(&self, other: &ServerDef) -> bool;                // command,args,env,binary,initialization_options
}
#[derive(Clone, Debug, PartialEq)] pub struct TableError { pub server: String, pub message: String }
#[derive(Clone, Debug, Default)] pub struct ServerTable { pub servers: BTreeMap<Arc<str>, ServerDef>, pub errors: Vec<TableError> }
impl ServerTable {
    pub fn defaults() -> &'static Value;            // parsed servers.json, OnceLock
    pub fn build(lsp_setting: Option<&Value>) -> ServerTable;   // merge defaults ← setting, validate
    pub fn get(&self, name: &str) -> Option<&ServerDef>;
}
pub fn merge_json(base: &mut Value, over: &Value);  // objects recursive, else replace
```
Rules: `language_id` — `One(s)` → `s`; `Map(m)` → `m[lucerna]`, else `lucerna`; no language → `One(s)` or first `languages` entry. `matches` — language present: `languages.contains(language)`; else file name equals an entry or the extension (case-insensitive) equals an entry. Valid entry needs (`command` or `binary.path`) and (`languages` or `file_types` non-empty); an invalid entry is left out and pushes `TableError { server, message }` where message is the serde error or `"missing \`command\`"` / `"missing \`languages\` or \`file_types\`"`. `servers.json` in this task: exactly the six Phase 3 servers with identical values (rust-analyzer outermost `Cargo.toml`; gopls ordered `go.work`,`go.mod`, probe `["version"]`; basedpyright/pyright `--stdio`, probe false, pyright `enabled: false`; typescript-language-server `--stdio`; clangd), each with `install` and `homepage`.

- [ ] **Step 1: Failing tests** (`tests/table.rs`): `defaults_parse_and_have_six_servers`; `defaults_match_phase3_values` (assert each field of the six against the old `builtin_adapters` values, written out in the test); `override_settings_only_keeps_default_launch` (setting `{"rust-analyzer":{"settings":{"a":1}}}` → command/args unchanged, settings `{"a":1}`); `user_adds_new_server` (`{"mylang-ls":{"command":"mylang-ls","file_types":["ml"]}}` → present, enabled); `arrays_replace_objects_merge` (user `args: ["x"]` replaces, `env` merges); `bad_entry_is_reported_and_skipped` (`{"bad":{"args":"--stdio"}}` and `{"empty":{"command":"x"}}` → two errors, others fine); `language_id_rules` (One, Map with `vue.js`→`vue`, fallback, no-language); `matches_by_language_and_file_type` (`rb` ext, `Gemfile` name, case-insensitive ext, language present skips file_types); `root_search_serde` (`"ordered"` → `NearestByMarker`); `install_for_os`.
- [ ] **Step 2:** `cargo test -p lucerna-lsp --test table` → FAIL (module missing).
- [ ] **Step 3:** Implement `table.rs`, `servers.json`, serde on `RootSearch`; export from `lib.rs`. Old adapter code still compiles untouched (removed in Task 2).
- [ ] **Step 4:** `cargo test -p lucerna-lsp` → PASS; clippy clean.
- [ ] **Step 5: Commit** `feat(lsp): server table data model with the Phase 3 servers as data`.

## Task 2: Variables, glob roots, and `ServerDef` everywhere in `lucerna-lsp`

**Files:** Create `crates/lucerna-lsp/src/vars.rs`; modify `adapters.rs`, `settings.rs`, `registry.rs`, `launcher.rs`, `test_support.rs`, `lib.rs`; tests `tests/vars.rs`, update `tests/{adapters.rs,registry.rs}`.

**Interfaces — Consumes:** Task 1. **Produces:**
```rust
// vars.rs
#[derive(Clone, Debug)] pub struct Vars { pub root: PathBuf, pub data_dir: PathBuf, pub npm_root: Option<PathBuf> }
#[derive(Debug, PartialEq)] pub enum VarError { Unknown(String), NpmRootUnavailable }
pub fn expand(text: &str, vars: &Vars) -> Result<String, VarError>;
pub fn expand_value(value: &Value, vars: &Vars) -> Result<Value, VarError>;   // every string inside
pub fn data_dir_for(base: &Path, server: &str, root: &Path) -> PathBuf;       // base/lsp/<server>/<12 hex>
pub fn uses_npm_root(def: &ServerDef) -> bool;
pub fn npm_root(launcher: &dyn Launcher) -> Option<PathBuf>;                  // `npm root -g`, trimmed
// launcher.rs
trait Launcher { fn output(&self, program: &str, args: &[&str]) -> Option<String>; /* which(program) then run, 5 s, login env, stdout on success */ }
// test_support.rs
impl FakeLauncher { pub fn with_output(self, program: &str, args: &[&str], stdout: &str) -> Self; }
// adapters.rs
pub fn find_root(def: &ServerDef, file: &Path, workspace_root: &Path) -> Option<PathBuf>;   // markers with * ? are globs over entry names
// settings.rs
pub fn select_servers<'a>(list: &[String], table: &'a ServerTable, language: Option<&str>, path: &Path) -> Vec<&'a ServerDef>;  // `...` = enabled && matches, by name order
pub fn resolve_launch(def: &ServerDef, root: &Path, launcher: &dyn Launcher, vars: &Vars) -> Result<LaunchSpec, StartError>;
// StartError::Failed for a VarError: message "unknown variable {name}" or the exact npm copy
// registry.rs
pub struct ServerKey { pub root: Arc<Path>, pub server: Arc<str> }
impl Registry {
    pub fn new(table: ServerTable) -> Registry;
    pub fn table(&self) -> &ServerTable;
    pub fn set_table(&mut self, table: ServerTable) -> SettingsDiff;   // restart if !launch_fields_eq, reconfigure if only settings differ, stop keys whose server vanished or got disabled
    pub fn keys_for(&self, language: Option<&str>, path: &Path, workspace_root: &Path, list: &[String]) -> Vec<ServerKey>;
}
pub struct SettingsDiff { pub restart: Vec<ServerKey>, pub reconfigure: Vec<(ServerKey, Value)>, pub stop: Vec<ServerKey> }
```
`resolve_launch`: program = `binary.path` (no probe) else `which(command)` (+ probe when `probe` is Some); args = `binary.arguments` else `args`, each `expand`ed; env = launcher env ⊕ `env` ⊕ `binary.env` (expanded); `initialization_options` expanded by the caller (Task 3). Remove `Adapter`, `Static`, `builtin_adapters`, `ServerSettings`, `ServersSettings`; `BinarySettings` stays.

- [ ] **Step 1: Failing tests:** `tests/vars.rs` — `expands_root_and_data_dir`, `unknown_variable_errors`, `npm_root_missing_errors`, `expand_value_walks_nested_strings`, `data_dir_is_stable_and_server_scoped`, `npm_root_reads_launcher_output`; `tests/adapters.rs` — `glob_marker_finds_sln` (`a.sln` two levels up → root), existing root tests ported to `ServerDef`; `tests/registry.rs` — existing tests ported (`select_default_python_list`, `resolve_*`, `settings_diff_*`), plus `select_by_file_type_without_language`, `set_table_stops_removed_server`, `jdtls_args_expand_data_dir`.
- [ ] **Step 2:** run → FAIL.
- [ ] **Step 3:** Implement; port every `lucerna-lsp` test to the new types (no behaviour change for the six servers).
- [ ] **Step 4:** `cargo test -p lucerna-lsp` → PASS (the workbench does not compile yet; that is Task 3 — do not commit a broken workspace: run `cargo check -p lucerna-lsp` and commit Tasks 2+3 together if needed, see Step 5).
- [ ] **Step 5: Commit** only after Task 3 makes the workspace build: Tasks 2 and 3 may share one commit `refactor(lsp): language server table replaces the Rust adapters`.

## Task 3: Workbench on the table: file types, language ids, variables, settings diff

**Files:** Modify `crates/lucerna-workbench/src/language_servers/{mod.rs,lifecycle.rs,sync.rs}`, `crates/lucerna-workbench/src/documents.rs` (only if `Opened` needs the path for language-less files — it already carries `path`), `crates/lucerna-workbench/src/trust.rs` (`server_binary_found` over the table); tests `tests/lsp_sync.rs`, `tests/trust.rs`.

**Interfaces — Consumes:** Tasks 1–2. **Produces:**
```rust
impl LanguageServers {
    pub fn table(&self) -> &ServerTable;
    pub fn table_errors(&self) -> &[TableError];
}
```
Behaviour: build `ServerTable::build(lsp)` where `lsp` = `get_trusted("lsp")` while untrusted and `get("lsp")` once trusted; rebuild on settings change and on trust change, apply `Registry::set_table` diff (restart / reconfigure / stop). `find_servers` runs for documents without a language too (selection list = `["..."]`, `enabled` = `lucerna.lsp.enabled` global). `didOpen` languageId = `def.lsp_language_id(doc.language_id)`. Launch: `Vars { root, data_dir: data_dir_for(<workbench data dir>, server, root), npm_root }` where `npm_root` is resolved once per `LanguageServers` in the background the first time a def `uses_npm_root`; `initialization_options` and `settings` are `expand_value`d before `initialize` / `workspace/configuration`. `server_binary_found` uses the trusted table.

- [ ] **Step 1: Failing tests** (`tests/lsp_sync.rs`, FakeLauncher): `language_less_file_starts_file_type_server` (open `x.rb` with a user table entry `{"ruby-lsp": …}` in the default table later — here use a user-defined `{"fake-rb":{"command":"fake-rb","file_types":["rb"]}}`) → `didOpen` with `languageId` from the entry; `file_type_and_language_share_one_server` (Review Focus 3); `user_defined_server_runs`; `workspace_defined_server_runs_only_when_trusted` (Review Focus 4); `settings_only_override_reconfigures` (Review Focus 1: `didChangeConfiguration`, no second launch); `disabling_server_stops_it`; `data_dir_variable_passed_to_server` (launch spec args contain the expanded path under the test data dir); `npm_root_failure_marks_failed` (exact message).
- [ ] **Step 2:** run → FAIL.
- [ ] **Step 3:** Implement; all existing workbench tests stay green.
- [ ] **Step 4:** `cargo test -p lucerna-workbench` full suite → PASS; `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean.
- [ ] **Step 5: Commit** (with Task 2) `refactor(lsp): language server table replaces the Rust adapters`.

## Task 4: The full default table

**Files:** Modify `crates/lucerna-lsp/assets/servers.json`; test `crates/lucerna-lsp/tests/table.rs`, `crates/lucerna-workbench/tests/lsp_table_languages.rs` (new).

Content: every row of spec §9 with the exact commands, root markers (globs where shown), `language_id` mappings (`vue.js`→`vue`, `c++`→`cpp`, `c#`→`csharp`, `objective-c`→`objective-c`, `shellscript`→`shellscript`), `file_types` (php/phtml; rb/rake/gemspec + `Gemfile`,`Rakefile`; java; kt/kts; swift; lua; ex/exs; zig/zon; dart; vue; c/h/cc/cpp/cxx/hpp/hh/m/mm; cs; hs/lhs; ml/mli; scala/sbt/sc; tf/tfvars; `Dockerfile` + dockerfile; yaml/yml; json/jsonc; css/scss/less; html/htm; sh/bash/zsh; toml; md/markdown; nix; gleam; erl/hrl), `install` (macOS strings from §9; `linux` where the same command works), `homepage`, `note` for ruby-lsp/elixir-ls/metals/clangd/sourcekit-lsp ("writes `.ruby-lsp/` in the project" etc.), alternates with `enabled: false` (phpactor, solargraph, kotlin-language-server, expert, roslyn-language-server, nixd, pyright). Vue entries per spec §5 including vtsls `settings` with `{npm_root}`; `relay: "vue-tsserver"` on vue-language-server. Sources: each server's own docs (homepage); cross-check with nvim-lspconfig (Apache-2.0); nothing copied from Helix.

- [ ] **Step 1: Failing tests:** `every_default_entry_is_complete` (command, languages-or-file_types, install, homepage); `one_enabled_server_per_language` (count enabled servers per `languages` entry ≤ 1 except `vue.js` = 2: vue-language-server + vtsls); `spec_rows_present` (list of 30 server names from §9 all present); workbench test `table_languages_resolve` (for each bundled language id in `LanguageRegistry::bundled()` that the table names, the id exists; ids absent from the bundled registry are allowed because extensions provide them).
- [ ] **Step 2:** run → FAIL.
- [ ] **Step 3:** Write the JSON.
- [ ] **Step 4:** PASS.
- [ ] **Step 5: Commit** `feat(lsp): default language server table for 30 languages`.

## Task 5: Vue `tsserver/request` relay

**Files:** Create `crates/lucerna-lsp/src/relay.rs`; modify `crates/lucerna-lsp/src/client.rs` (`ServerEvent::Notification`), `crates/lucerna-workbench/src/language_servers/pump.rs`; tests `crates/lucerna-lsp/tests/relay.rs`, `crates/lucerna-workbench/tests/lsp_vue.rs` (new).

**Interfaces — Produces:**
```rust
ServerEvent::Notification { method: String, params: Value }   // every server notification the client does not handle itself
pub struct TsRequest { pub id: Value, pub command: String, pub args: Value }
pub fn parse_tsserver_request(params: &Value) -> Vec<TsRequest>;
pub fn execute_command_params(req: &TsRequest) -> lsp_types::ExecuteCommandParams;   // command "typescript.tsserverRequest"
pub fn tsserver_response(id: &Value, body: Option<Value>) -> Value;                    // params of `tsserver/response`
pub fn response_body(result: &Value) -> Option<Value>;                                 // `result.body` of executeCommand
```
- [ ] **Step 1: Verify shapes upstream.** Read `vuejs/language-tools` (MIT) server source for `tsserver/request` / `tsserver/response` and nvim-lspconfig `lsp/vue_ls.lua` (Apache-2.0). Record the exact params shape and `arguments` in this task's report; if it differs from spec §5, update spec §5 text in the same commit and implement upstream's shape.
- [ ] **Step 2: Failing tests:** `relay.rs` unit tests for parse/build per the verified shape (batch of two requests); workbench `lsp_vue.rs` with two FakeServers on a `.vue` file: `relay_forwards_to_vtsls_and_answers` (vue fake sends `tsserver/request` after `didOpen`; vtsls fake receives `workspace/executeCommand` with `typescript.tsserverRequest`; vue fake receives `tsserver/response` with the same id and the body); `relay_without_typescript_server_answers_null` (Review Focus 5); `relay_falls_back_to_typescript_language_server`.
- [ ] **Step 3:** Implement; pump routes `Notification { method: "tsserver/request" }` from a server whose def has `relay: VueTsserver` to the running `vtsls` (else `typescript-language-server`) key with the same root; requests run as background tasks and must not block other events.
- [ ] **Step 4:** PASS; full workbench suite green.
- [ ] **Step 5: Commit** `feat(lsp): Vue tsserver relay`.

## Task 6: Health core, `luc lsp health`, and install hints in the ⚡ popover

**Files:** Create `crates/lucerna-lsp/src/health.rs`, `crates/lucerna/src/lsp_cli.rs`; modify `crates/lucerna/src/main.rs`, `crates/lucerna-workbench/src/lsp_status.rs`; tests `crates/lucerna-lsp/tests/health.rs`, `crates/lucerna/tests/lsp_cli.rs` (insta), `crates/lucerna-workbench/tests/lsp_status.rs`.

**Interfaces — Produces:**
```rust
#[derive(Clone, Debug, PartialEq)] pub enum HealthStatus { Found(PathBuf), NotFound, Disabled }
#[derive(Clone, Debug, PartialEq)] pub struct HealthRow { pub language: String, pub server: Arc<str>, pub status: HealthStatus, pub install: Option<String>, pub note: Option<String> }
pub fn health(table: &ServerTable, launcher: &dyn Launcher, os: &str) -> Vec<HealthRow>;   // one row per (language, server); language = each `languages` entry, else first file_type; sorted by language then enabled first
pub fn format_table(rows: &[HealthRow]) -> String;   // columns: Language, Server, Status (`✓ <path>` / `✗ not found` / `disabled`), Install
pub fn current_os() -> &'static str;                  // "macos" | "linux" | "windows"
```
`luc lsp health`: `Command::Lsp(LspArgs)` with subcommand `Health`; builds the table from the User settings file (same path the app uses) via `SettingsStore` User layer, `ProcessLauncher`, prints `format_table`, exit 0. Popover: a `NotFound` row adds a second line with the install command (code font, muted) and a `Copy` text button writing it to the clipboard; each `TableError` is a row `Invalid \`lsp.<name>\`: <reason>`; footer gets `Show All Languages…` running `lucerna.lsp.health`.

- [ ] **Step 1: Failing tests:** `health.rs` — `found_not_found_disabled` (FakeLauncher), `language_rows_and_order`, `format_table_columns`; CLI — `lsp_health_snapshot` (FakeLauncher injected via a hidden `--fake-path` test hook or by constructing through the library function `run_health(table, launcher, out)` — test the function, not the binary, and snapshot its output with insta); workbench — `not_found_row_shows_install_and_copy` (clipboard holds the exact command), `table_error_row_shown` (Review Focus 2), `footer_opens_health` (command dispatched).
- [ ] **Step 2:** FAIL. **Step 3:** Implement. **Step 4:** PASS.
- [ ] **Step 5: Commit** `feat: language server health — CLI and popover install hints`.

## Task 7: Health tab

**Files:** Create `crates/lucerna-workbench/src/lsp_health.rs`; modify `editor_groups/{model.rs,view.rs}`, `workbench.rs` (command registration only), `lib.rs`; test `tests/lsp_health.rs`.

**Interfaces — Consumes:** Task 6 `health`, `HealthRow`. **Produces:** `TabItem::LspHealth`; command `lucerna.lsp.health`; debug selectors `lsp-health`, `lsp-health-summary`, `lsp-health-row-{i}`, `lsp-health-copy-{i}`, `lsp-health-refresh`.

UI per spec §7: one tab, not persisted; header rows per language (reuse the `excerpt_list` file-header look: `editor.subheader.background`, 1 px border), rows with server name, status (`✓ path` in `text.muted`, `✗ not found` in `warning`, `disabled` muted), install command + `Copy`, `note` muted under the row; summary row `"{found} of {total} servers found"` and a refresh `bar_icon_button`. `health()` runs in `background_spawn`; results cached until refresh or settings change; uses the trusted table rules of Task 3.

- [ ] **Step 1: Failing tests:** `health_tab_lists_every_language`, `summary_counts`, `copy_puts_install_command_on_clipboard`, `refresh_recomputes`, `not_persisted_in_workspace_state`, `untrusted_workspace_server_not_listed_as_found`.
- [ ] **Step 2:** FAIL. **Step 3:** Implement. **Step 4:** PASS; full suite green.
- [ ] **Step 5: Commit** `feat(workbench): language server Health tab`.

## Task 8: Docs, verification, real-server check, bundle

**Files:** Modify `docs/superpowers/specs/2026-11-10-lucerna-phase3-lsp-design.md` (D3, out-of-scope line, §3.1 adapters paragraph, §8 settings table per spec §8), `docs/superpowers/specs/2026-11-11-lsp-server-table-design.md` (status line → "implemented (commit …)"; add `note` to the §3.1 field table), README feature list if it lists LSP languages.

- [ ] **Step 1:** Doc edits in Vietnamese matching existing style.
- [ ] **Step 2:** Full verification: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo test --workspace`, `cargo deny check` → all clean; `cargo test -p lucerna-lsp -- --ignored rust_analyzer` → PASS.
- [ ] **Step 3:** Real check on the dev machine: `luc lsp health` lists the table (rust-analyzer found via user settings or PATH, clangd found); open a C file with clangd and a Rust file; record results.
- [ ] **Step 4:** `scripts/bundle-app.sh --install`.
- [ ] **Step 5: Commit** `docs: language server table status; Phase 3 D3 updated`.
