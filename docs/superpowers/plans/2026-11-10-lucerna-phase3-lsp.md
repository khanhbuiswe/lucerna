# Phase 3 (LSP) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Read-only code intelligence over LSP — definition / references / hover / diagnostics / document & workspace symbols / breadcrumbs — from language servers found on PATH, behind Zed-style folder trust.

**Architecture:** A new gpui-free crate `lucerna-lsp` holds the JSON-RPC client (transport behind a trait, so tests use an in-process fake server), adapters, registry, document sync, diagnostics store and symbol conversion. `lucerna-syntax` gains a shared `SymbolTree` type and `outline.scm` support. `lucerna-workbench` gets one bridge entity (`LanguageServers`) plus small UI modules that reuse `ui.rs` / picker components.

**Tech Stack:** Rust 2024, gpui (pinned Zed rev), `lsp-types 0.97` (MIT), `pulldown-cmark 0.13` (MIT), `futures`, `crossbeam-channel`, `globset`.

**Spec:** `docs/superpowers/specs/2026-11-10-lucerna-phase3-lsp-design.md` (section numbers below, `§n`, refer to it). Research: `docs/research/2026-11-10-lsp.md`.

## Global Constraints

- Only `lucerna-workbench` depends on gpui. `lucerna-lsp` must not depend on gpui or tokio.
- Zed's `lsp`, `project`, `editor`, `language`, `languages`, `outline`, `breadcrumbs`, `project_symbols`, `lsp_locations` crates are GPL-3: never copy their code. Icons come from Lucide (ISC) with the license file kept as for existing icons.
- License MIT OR Apache-2.0; `cargo deny check` must pass after every dependency change.
- No blocking work on the UI thread: process spawn, `which`, probes, login-shell env, file reads and JSON parsing run off it. Background closures are wrapped in `tasks::guarded` / `catch_panic`; transport threads use `tasks`-style guards in `lucerna-lsp` (own helper, same behaviour: log `tracing::error!` and treat as exit).
- Paths are absolute and normalised through `Workbench::absolute_path`; external changes arrive via `Workbench::on_worktree_event`.
- Platform code (login shell, process kill) lives behind `cfg(unix)` / `cfg(windows)` in `lucerna-lsp/src/env.rs` and `process.rs` only.
- Cargo: `export PATH="/opt/homebrew/opt/rustup/bin:$PATH"`. Keep ≥ 1.2 GB free disk (`df -h .` before builds); never build `--release` during development.
- Commit on `main`, staging files by explicit path; trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Timeouts: request 120 s; hover and ⌘-hover 10 s; `initialize` 60 s; shutdown 5 s then `exit`, 1 s, kill; probe 5 s; login shell 5 s.
- Defaults: `editor.hover.delay` 300, `editor.hover.hidingDelay` 300, `editor.hover.sticky` true, `editor.hover.enabled` true; diagnostics batch 50 ms; `documentSymbol` debounce 300 ms (empty result cached 2 s); `#` debounce 100 ms, max 100 results; nav history 50 entries, same-file threshold 10 rows; log 2000 lines; `didChangeWatchedFiles` batch 100 ms; no `didOpen` above `lucerna.largeFile.*` or with a line > 20 000 chars.
- Copy strings are exact as written in the spec (§4, §5, §7).

## Review Focus

1. **Agent rewrites a file many times per second while the server is busy** → diagnostics with a stale `version` are dropped, the newest text wins, no panic from out-of-range positions (Task 6 `stale_version_dropped`, `out_of_range_clamped`; Task 11 `stale_version_ignored`).
2. **Server binary on PATH is a rustup proxy without the component, or exits right after start** → `NotFound` / `Failed` with stderr tail, never a hang, other servers unaffected (Task 5 `resolve_probe_failure_is_not_found`; Task 8 `crash_marks_failed_and_clears_diagnostics`).
3. **Non-ASCII text (emoji, CJK, tabs) on lines with diagnostics, hover ranges, definitions** → UTF-16 ↔ byte columns round-trip and underlines land on the right glyphs (Task 1 tests; Task 6 `resolve_open_converts_raw_utf16`; Task 11 `emoji_line_mark_columns`).
4. **Two tabs (or two groups) on the same file, then closing one** → one `didOpen`, no `didClose` until the last view goes, diagnostics shown in both (Task 8 `two_tabs_one_did_open_close_after_last`).
5. **Untrusted folder whose `.lucerna/settings.json` sets `lsp.<server>.binary.path`** → nothing is spawned before trust; after trust the workspace value is honoured (Task 9 `workspace_lsp_binary_path_ignored_until_trusted`).

---

## File map

**New crate `crates/lucerna-lsp/`**
- `Cargo.toml`, `src/lib.rs`
- `src/position.rs` — `PositionEncoding`, UTF-16/UTF-8 ↔ `Point`, path ↔ URI.
- `src/framing.rs` — `Content-Length` read/write.
- `src/transport.rs` — `Transport`, `Inbox` traits, `LaunchSpec`.
- `src/process.rs` — process transport (threads, kill).
- `src/client.rs` — `LanguageServer`, `RequestFuture`, `ServerEvent`, `LspError`, auto-replies, shutdown.
- `src/capabilities.rs` — client capabilities and `InitializeParams` builder.
- `src/env.rs` — login-shell env, `which`.
- `src/launcher.rs` — `Launcher` trait, `ProcessLauncher`.
- `src/adapters.rs` — `Adapter` trait, built-in adapters, root search.
- `src/settings.rs` — `ServerSettings`, `BinarySettings`, language-server list resolution (`!`, `...`).
- `src/registry.rs` — `ServerKey`, `Slot`, `Registry`.
- `src/sync.rs` — `DocumentSync`, `WatchedFiles`.
- `src/diagnostics.rs` — `Diagnostic`, `Severity`, `DiagnosticStore`.
- `src/locations.rs` — `ResolvedLocation`, resolving LSP ranges (reads files).
- `src/symbols.rs` — LSP symbols → `lucerna_syntax::SymbolTree`.
- `src/test_support.rs` (feature `test-support`) — `FakeServer`, `FakeLauncher`.
- `tests/*.rs`.

**`crates/lucerna-syntax/`**: `src/symbols.rs` (new: `Symbol`, `SymbolKind`, `SymbolTree`), `src/outline.rs` (new: run `outline.scm`), `src/registry.rs` (read `outline.scm`).

**`crates/lucerna-settings/`**: `src/store.rs` (`get_trusted`), `assets/default-settings.json`, `assets/default-keybindings.json`.

**`crates/lucerna-theme/`**: `src/lib.rs`, `src/schema.rs` (new colors).

**`crates/lucerna-vim/`**: `src/lib.rs` (`Action::RunCommand`), `src/state.rs` (`gd`, `gD`, `gh`, `]d`, `[d`, `⌃o`, `⌃i`).

**`crates/lucerna-workbench/src/`** (new unless marked):
- `language_servers.rs` — bridge entity.
- `trust.rs` — `TrustStore`, modal, title-bar button.
- `nav_history.rs`.
- `lsp_status.rs` — ⚡ button, popover, progress, log tab body.
- `diagnostics_status.rs` — status-bar counts/message, F8 commands.
- `code_view/diagnostics.rs`, `code_view/link.rs`; modify `code_view/mod.rs`, `code_view/element.rs`, `code_view/element/paint.rs`.
- `markdown.rs`, `hover.rs`, `goto.rs`, `locations_picker.rs`.
- `excerpt_list/{mod.rs,model.rs,highlight.rs,render.rs}` — extracted from `search_view`; modify `search_view/*`.
- `references_view.rs`.
- `symbols/{mod.rs,source.rs,quick_open.rs,breadcrumbs.rs}`; modify `quick_open/{query.rs,view.rs,mod.rs}`.
- Modify: `documents.rs` (events), `workbench.rs`, `editor_groups/{model.rs,view.rs}` (tab kinds, breadcrumbs row), `layout/title_bar.rs`, `layout/status_bar.rs` (none expected), `icons.rs`, `vim.rs`, `lib.rs`, `keys.rs` (context keys).
- Tests: `crates/lucerna-workbench/tests/{lsp_sync.rs,trust.rs,diagnostics.rs,hover.rs,goto.rs,references.rs,symbols.rs,lsp_status.rs,nav_history.rs}` plus `tests/support/lsp.rs` (shared fake-server helpers).

---

## Task 1: `lucerna-lsp` crate, position and URI helpers

**Files:** Create `crates/lucerna-lsp/{Cargo.toml,src/lib.rs,src/position.rs}`; modify root `Cargo.toml` (workspace deps `lsp-types = "0.97"`, `pulldown-cmark = { version = "0.13", default-features = false }`, `lucerna-lsp` path); `deny.toml` only if `cargo deny` requires.

**Interfaces — Produces:**
```rust
pub enum PositionEncoding { Utf8, Utf16 }               // Copy, Eq
impl PositionEncoding { pub fn from_lsp(kind: Option<&lsp_types::PositionEncodingKind>) -> Self } // None/unknown → Utf16
pub fn to_lsp(snapshot: &BufferSnapshot, p: Point, enc: PositionEncoding) -> lsp_types::Position
pub fn from_lsp(snapshot: &BufferSnapshot, p: lsp_types::Position, enc: PositionEncoding) -> Point
pub fn column_from_lsp(line: &str, character: u32, enc: PositionEncoding) -> u32   // byte column, clamped, never splits a char
pub fn column_to_lsp(line: &str, byte_column: u32, enc: PositionEncoding) -> u32
pub fn range_from_lsp(snapshot: &BufferSnapshot, r: lsp_types::Range, enc) -> Range<Point>
pub fn path_to_uri(path: &Path) -> lsp_types::Uri
pub fn uri_to_path(uri: &lsp_types::Uri) -> Option<PathBuf>   // file:// only, percent-decoded
```
`Point.column` is a UTF-8 byte offset (`lucerna_text::Point`). Rows past the end clamp to the last row's end; columns past the line end clamp to its end (line text excludes `\n`/`\r\n`).

- [ ] **Step 1: Write failing tests** in `src/position.rs` `#[cfg(test)]`:
  - `utf16_counts_surrogate_pairs`: line `"a😀b"`: `column_to_lsp(line, 5, Utf16) == 3` (a=1, 😀=2), `column_from_lsp(line, 3, Utf16) == 5`.
  - `cjk_round_trip`: `"日本語x"`, every char boundary round-trips in both encodings.
  - `utf8_is_identity`: `column_to_lsp("héllo", 3, Utf8) == 3`.
  - `out_of_range_clamps`: row 99 of a 2-line snapshot → last row end; character 999 → line end; a UTF-16 position inside a surrogate pair → start of that char.
  - `crlf_line_end_excluded`: text `"ab\r\ncd"`, `from_lsp(row 0, char 5)` → `Point{row:0,column:2}`.
  - `uri_round_trip`: `/tmp/a b/ü.rs` → `file:///tmp/a%20b/%C3%BC.rs` → same path; `uri_to_path("untitled:x") == None`.
- [ ] **Step 2:** `cargo test -p lucerna-lsp position` → FAIL (not defined).
- [ ] **Step 3:** Implement. Use `url::Url::from_file_path` / `to_file_path` for URIs and parse into `lsp_types::Uri` via `str::parse`.
- [ ] **Step 4:** `cargo test -p lucerna-lsp position` → PASS; `cargo deny check` → ok.
- [ ] **Step 5: Commit** `feat(lsp): new lucerna-lsp crate with position and URI conversion`.

## Task 2: Framing and process transport

**Files:** Create `src/framing.rs`, `src/transport.rs`, `src/process.rs`; test `tests/process.rs` (`#![cfg(unix)]`) with fixture `tests/fixtures/echo_lsp.sh` (a `sh` script that reads one framed message and writes it back framed, then writes `boom` to stderr and exits 3).

**Interfaces — Produces:**
```rust
// framing.rs
pub fn write_message(w: &mut impl Write, body: &str) -> io::Result<()>       // "Content-Length: N\r\n\r\n" + body, flush
pub fn read_message(r: &mut impl BufRead) -> io::Result<Option<String>>      // Ok(None) at clean EOF; InvalidData on bad/missing Content-Length
// transport.rs
pub trait Transport: Send + Sync + 'static { fn send(&self, body: String); fn kill(&self); }
pub trait Inbox: Send + Sync + 'static {
    fn message(&self, body: String);           // called on the reader thread
    fn stderr(&self, line: String);
    fn closed(&self, exit_code: Option<i32>);  // exactly once
}
#[derive(Clone, Debug)] pub struct LaunchSpec { pub server: String, pub program: PathBuf, pub args: Vec<String>, pub env: HashMap<String,String>, pub cwd: PathBuf }
// process.rs
pub fn spawn_process(spec: &LaunchSpec, inbox: Arc<dyn Inbox>) -> io::Result<Arc<dyn Transport>>
```
Threads named `lsp-read-<server>`, `lsp-write-<server>`, `lsp-stderr-<server>`; writer fed by an unbounded `crossbeam_channel`; reader calls `inbox.closed` after EOF/InvalidData and child `wait`. `env` replaces the child environment entirely (`env_clear` then set). Panics inside a thread are caught and reported as `closed(None)`.

- [ ] **Step 1: Failing tests.** `framing.rs` unit tests: `reads_two_messages_in_one_buffer`, `accepts_content_type_header_any_order`, `clean_eof_is_none`, `missing_length_is_invalid_data`, `body_of_64k_bytes`. `tests/process.rs`: `echo_round_trip_then_stderr_and_exit_code` (inbox records `message` = sent body, `stderr` = `"boom"`, `closed(Some(3))`); `kill_closes` (spawn `sh -c 'sleep 30'`, `kill()`, `closed` within 2 s).
- [ ] **Step 2:** run → FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** `cargo test -p lucerna-lsp` → PASS.
- [ ] **Step 5: Commit** `feat(lsp): JSON-RPC framing and process transport`.

## Task 3: `LanguageServer` client, fake server

**Files:** Create `src/client.rs`, `src/capabilities.rs`, `src/test_support.rs` (feature `test-support`, also enabled in this crate's dev-dependencies); test `tests/client.rs`.

**Interfaces — Consumes:** Task 2 `Transport`, `Inbox`. **Produces:**
```rust
pub enum LspError { Timeout, Closed, Response { code: i64, message: String }, Decode(String) }
pub struct RequestFuture<T>  // impl Future<Output = Result<T, LspError>>; Drop before completion sends $/cancelRequest {id}
pub enum ServerEvent {
    Diagnostics(lsp_types::PublishDiagnosticsParams),
    Progress { token: String, title: Option<String>, message: Option<String>, percentage: Option<u32>, done: bool },
    Log(LogLine),
    ShowMessage { kind: lsp_types::MessageType, message: String },
    WatchedFilesRegistered { id: String, watchers: Vec<lsp_types::FileSystemWatcher> },
    WatchedFilesUnregistered { id: String },
    Exited(Option<i32>),
}
pub struct LogLine { pub kind: LogKind, pub text: String }   // LogKind: Stderr | Server | Lifecycle
pub struct LanguageServer { /* … */ }
impl LanguageServer {
    pub fn start(name: &str, launch: impl FnOnce(Arc<dyn Inbox>) -> io::Result<Arc<dyn Transport>>, settings: serde_json::Value)
        -> io::Result<(Arc<LanguageServer>, futures::channel::mpsc::UnboundedReceiver<ServerEvent>)>;
    pub fn initialize(self: &Arc<Self>, params: lsp_types::InitializeParams) -> impl Future<Output = Result<(), LspError>>; // 60 s; sends `initialized` then didChangeConfiguration{settings}
    pub fn request<R: lsp_types::request::Request>(&self, params: R::Params) -> RequestFuture<R::Result>;   // 120 s
    pub fn request_with_timeout<R: Request>(&self, params: R::Params, timeout: Duration) -> RequestFuture<R::Result>;
    pub fn notify<N: lsp_types::notification::Notification>(&self, params: N::Params);
    pub fn capabilities(&self) -> Option<Arc<lsp_types::ServerCapabilities>>;
    pub fn encoding(&self) -> PositionEncoding;
    pub fn name(&self) -> &str;
    pub fn server_version(&self) -> Option<String>;
    pub fn set_settings(&self, settings: serde_json::Value);   // stores + sends workspace/didChangeConfiguration
    pub fn is_closed(&self) -> bool;
    pub fn shutdown(self: &Arc<Self>) -> std::thread::JoinHandle<()>;   // shutdown(5 s) → exit → wait 1 s → kill; idempotent
}
pub fn initialize_params(root: &Path, initialization_options: Option<Value>) -> lsp_types::InitializeParams; // capabilities.rs, §3.1 list exactly
// test_support.rs
pub struct FakeServer { /* … */ }
impl FakeServer {
    pub fn new() -> FakeServer;                                   // answers `initialize` with default capabilities (full sync, definition, references, hover, documentSymbol, workspaceSymbol, positionEncoding utf-16)
    pub fn capabilities(self, caps: lsp_types::ServerCapabilities) -> Self;
    pub fn on_request(self, method: &str, f: impl Fn(Value) -> Result<Value, (i64, String)> + Send + Sync + 'static) -> Self;
    pub fn on_notification(self, method: &str, f: impl Fn(Value, &FakePeer) + Send + Sync + 'static) -> Self; // FakePeer::notify(method, params), FakePeer::request(method, params)
    pub fn hang_on(self, method: &str) -> Self;                   // never answers
    pub fn received(&self) -> Vec<(String, Value)>;               // method + params of every message the client sent, in order
    pub fn crash(&self, code: Option<i32>);
    pub fn launch(&self, inbox: Arc<dyn Inbox>) -> io::Result<Arc<dyn Transport>>; // synchronous: send() answers inline, no threads
}
```
Server→client request auto-replies exactly per §3.1 table; `workspace/configuration` answers `settings` (whole object for no `section`, else the dotted path, else `null`). Timeouts are enforced by one watchdog thread per server (tick 100 ms) that fails expired requests with `Timeout`. `$/progress` is parsed into `Progress` (`WorkDoneProgress::Begin/Report/End`); `window/logMessage` → `Log(Server)`; stderr lines → `Log(Stderr)`.

- [ ] **Step 1: Failing tests** (`tests/client.rs`, using `FakeServer` + `futures::executor::block_on`):
  - `initialize_sends_capabilities_then_initialized_and_configuration`: `received()` methods == `["initialize","initialized","workspace/didChangeConfiguration"]`; `initialize` params have `general.positionEncodings == ["utf-8","utf-16"]`, `textDocument.hover.contentFormat == ["markdown","plaintext"]`, no `completion`.
  - `request_resolves_by_id` (two concurrent requests, answers in reverse order).
  - `dropping_request_sends_cancel`: `hang_on("textDocument/hover")`; drop the future; `received()` last == `("$/cancelRequest", {"id": <id>})`.
  - `timeout_fails_request`: `request_with_timeout(.., 200ms)` on a hung method → `Err(Timeout)` within 1 s.
  - `closed_fails_pending_and_emits_exited`: hung request, `crash(Some(1))` → `Err(Closed)`, event `Exited(Some(1))`.
  - `auto_replies`: peer requests `workspace/configuration` with items `[{section:"a.b"},{}]` and settings `{"a":{"b":1}}` → response `[1, {"a":{"b":1}}]`; `window/workDoneProgress/create` → `null`; `foo/bar` → error code `-32601`.
  - `register_watched_files_emits_event`.
  - `progress_and_log_events`.
  - `shutdown_sends_shutdown_then_exit`.
- [ ] **Step 2:** run → FAIL. **Step 3:** implement. **Step 4:** `cargo test -p lucerna-lsp` → PASS.
- [ ] **Step 5: Commit** `feat(lsp): language server client with cancel-on-drop, timeouts and fake server`.

## Task 4: Environment, launcher, adapters

**Files:** Create `src/env.rs`, `src/launcher.rs`, `src/adapters.rs`; extend `src/test_support.rs` with `FakeLauncher`; test `tests/adapters.rs`.

**Interfaces — Produces:**
```rust
// env.rs
pub fn login_shell_env() -> HashMap<String,String>;   // unix: `$SHELL -l -i -c 'printf "\0LUCERNA_ENV\0"; env -0'` 5 s timeout, parse after marker; else/err: std::env::vars()
pub fn which(name: &str, path_var: &str) -> Option<PathBuf>;   // first executable file on PATH
// launcher.rs
pub trait Launcher: Send + Sync + 'static {
    fn env(&self) -> HashMap<String,String>;          // may block; cached after first call
    fn which(&self, name: &str) -> Option<PathBuf>;
    fn probe(&self, program: &Path, args: &[&str]) -> bool;   // exit status success within 5 s, stdin closed
    fn launch(&self, spec: &LaunchSpec, inbox: Arc<dyn Inbox>) -> io::Result<Arc<dyn Transport>>;
}
pub struct ProcessLauncher;  // impl Launcher using env.rs + process.rs
// adapters.rs
pub enum RootSearch { Nearest, Outermost }
pub trait Adapter: Send + Sync + 'static {
    fn name(&self) -> &'static str;
    fn language_ids(&self) -> &'static [&'static str];
    fn binary(&self) -> &'static str;
    fn args(&self) -> &'static [&'static str];
    fn root_markers(&self) -> &'static [&'static str];
    fn root_search(&self) -> RootSearch { RootSearch::Nearest }
    fn probe_args(&self) -> &'static [&'static str] { &["--version"] }   // empty = do not probe
    fn lsp_language_id(&self, lucerna_id: &str) -> String { lucerna_id.to_string() }
    fn initialization_options(&self) -> Option<Value> { None }
    fn enabled_by_default(&self) -> bool { true }
}
pub fn builtin_adapters() -> Vec<Arc<dyn Adapter>>;   // rust-analyzer, gopls, basedpyright, pyright(enabled_by_default=false), typescript-language-server, clangd — values from spec §3.1 table; gopls probe `version`; pyright & basedpyright probe empty
pub fn find_root(adapter: &dyn Adapter, file: &Path, workspace_root: &Path) -> PathBuf;  // never above workspace_root; none found → workspace_root; file outside root → file's parent
// test_support.rs
pub struct FakeLauncher;  // new(); with_server(binary_name: &str, server: FakeServer); missing binaries → which() None; launches(): Vec<LaunchSpec>
```
- [ ] **Step 1: Failing tests:** `rust_root_is_outermost_cargo_toml` (root/Cargo.toml + root/crates/a/Cargo.toml, file in a → root); `go_prefers_go_work_over_go_mod` (`go.work` at root, `go.mod` in sub → root); `ts_nearest_package_json`; `root_never_escapes_workspace`; `which_finds_executable_only` (non-executable file skipped; `#[cfg(unix)]`); `login_env_falls_back_on_bad_shell` (`SHELL=/nonexistent` via a helper taking the shell path: expose `pub fn shell_env_from(shell: &Path) -> Option<HashMap<..>>`) ; `builtin_adapter_table` (names, language ids, args exactly as spec).
- [ ] **Step 2:** FAIL. **Step 3:** implement. **Step 4:** PASS.
- [ ] **Step 5: Commit** `feat(lsp): adapters for PATH servers, root search, login-shell env`.

## Task 5: Server settings, selection and registry

**Files:** Create `src/settings.rs`, `src/registry.rs`; test `tests/registry.rs`.

**Interfaces — Consumes:** Tasks 3–4. **Produces:**
```rust
// settings.rs
#[derive(Deserialize, Default, Clone, PartialEq)] pub struct BinarySettings { pub path: Option<PathBuf>, pub arguments: Option<Vec<String>>, pub env: Option<HashMap<String,String>> }
#[derive(Deserialize, Default, Clone, PartialEq)] pub struct ServerSettings { pub binary: Option<BinarySettings>, pub initialization_options: Option<Value>, pub settings: Option<Value> }
pub type ServersSettings = HashMap<String, ServerSettings>;     // value of the `lsp` setting
pub fn select_servers(list: &[String], adapters: &[Arc<dyn Adapter>], language_id: &str) -> Vec<Arc<dyn Adapter>>;
//   list order wins; "!name" removes; "..." = remaining adapters for the language with enabled_by_default; unknown names ignored
pub fn resolve_launch(adapter: &dyn Adapter, settings: Option<&ServerSettings>, root: &Path, launcher: &dyn Launcher) -> Result<LaunchSpec, StartError>;
pub enum StartError { NotFound { message: String }, Failed { message: String, stderr_tail: Vec<String> } }
//   NotFound message exactly: `{binary} not found on PATH. Install it or set "lsp": { "{name}": { "binary": { "path": … } } }`
// registry.rs
#[derive(Clone, Hash, Eq, PartialEq, Debug)] pub struct ServerKey { pub root: Arc<Path>, pub server: &'static str }
pub enum Slot { Starting, Running(Arc<LanguageServer>), Failed { message: String, stderr_tail: Vec<String> }, NotFound { message: String }, Stopped }
pub struct Registry { /* adapters, slots, settings */ }
impl Registry {
    pub fn new(adapters: Vec<Arc<dyn Adapter>>) -> Registry;
    pub fn keys_for(&self, language_id: &str, path: &Path, workspace_root: &Path, list: &[String]) -> Vec<ServerKey>;
    pub fn adapter(&self, server: &str) -> Option<Arc<dyn Adapter>>;
    pub fn slot(&self, key: &ServerKey) -> Option<&Slot>;
    pub fn set_slot(&mut self, key: ServerKey, slot: Slot);
    pub fn running(&self) -> impl Iterator<Item = (&ServerKey, &Arc<LanguageServer>)>;
    pub fn keys(&self) -> impl Iterator<Item = &ServerKey>;
    pub fn settings(&self, server: &str) -> Option<&ServerSettings>;
    pub fn set_settings(&mut self, new: ServersSettings) -> SettingsDiff;
    pub fn remove(&mut self, key: &ServerKey) -> Option<Slot>;
}
pub struct SettingsDiff { pub restart: Vec<ServerKey>, pub reconfigure: Vec<(ServerKey, Value)> }
//   restart when binary or initialization_options changed; reconfigure (settings only) otherwise
```
`resolve_launch`: `binary.path` → use as is (no probe), args = `binary.arguments` or adapter args; else `which(adapter.binary())` on `launcher.env()["PATH"]`, then `probe` when `probe_args` non-empty (failure → `NotFound` with the same message); env = launcher env overlaid with `binary.env`; cwd = root.

- [ ] **Step 1: Failing tests:** `select_default_python_list` (`["basedpyright","!pyright","..."]` → `[basedpyright]`), `select_explicit_pyright_first`, `select_bang_removes_from_dots`, `resolve_uses_binary_path_without_probe`, `resolve_not_found_message_exact`, `resolve_probe_failure_is_not_found`, `binary_env_overlays_login_env`, `settings_diff_restart_vs_reconfigure`, `keys_for_two_crates_share_outer_root`.
- [ ] **Step 2–4:** FAIL → implement → PASS.
- [ ] **Step 5: Commit** `feat(lsp): server settings, selection and registry`.

## Task 6: Document sync, watched files, diagnostics store, locations

**Files:** Create `src/sync.rs`, `src/diagnostics.rs`, `src/locations.rs`; tests `tests/sync.rs`, `tests/diagnostics.rs`.

**Interfaces — Produces:**
```rust
// sync.rs
pub struct DocumentSync;  // per (path, ServerKey): version, last snapshot
impl DocumentSync {
    pub fn open(&mut self, path: &Path, lsp_language_id: &str, snapshot: &BufferSnapshot, servers: &[(ServerKey, Arc<LanguageServer>)]);   // version 0, full text, once per (path, key)
    pub fn change(&mut self, path: &Path, snapshot: &BufferSnapshot, servers: &[(ServerKey, Arc<LanguageServer>)]); // version+1; Full → whole text; Incremental → one change replacing 0:0..end-of-old; None → nothing
    pub fn close(&mut self, path: &Path, servers: &[(ServerKey, Arc<LanguageServer>)]);
    pub fn forget_server(&mut self, key: &ServerKey);
    pub fn version(&self, path: &Path, key: &ServerKey) -> Option<i32>;
    pub fn is_open(&self, path: &Path, key: &ServerKey) -> bool;
}
pub fn too_large_for_lsp(snapshot: &BufferSnapshot, max_bytes: u64, max_lines: u32) -> bool;   // also true if any line > 20_000 chars
pub struct WatchedFiles;
impl WatchedFiles {
    pub fn register(&mut self, key: ServerKey, id: String, watchers: Vec<lsp_types::FileSystemWatcher>);   // relative patterns resolved against baseUri; plain globs against key.root
    pub fn unregister(&mut self, key: &ServerKey, id: &str);
    pub fn events_for(&self, changes: &[(PathBuf, lsp_types::FileChangeType)]) -> HashMap<ServerKey, Vec<lsp_types::FileEvent>>; // honours WatchKind
    pub fn forget_server(&mut self, key: &ServerKey);
}
// diagnostics.rs
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)] pub enum Severity { Error, Warning, Information, Hint }  // Error sorts first
#[derive(Clone, Debug, PartialEq)] pub struct Diagnostic { pub range: Range<Point>, pub severity: Severity, pub message: String, pub source: Option<String>, pub code: Option<String>, pub code_href: Option<String>, pub unnecessary: bool, pub deprecated: bool }
pub struct DiagnosticStore;
impl DiagnosticStore {
    pub fn publish(&mut self, key: &ServerKey, params: lsp_types::PublishDiagnosticsParams, encoding: PositionEncoding, open: Option<(&BufferSnapshot, i32 /*current version for key*/ )>) -> Option<PathBuf>; // None when dropped as stale (params.version != current)
    pub fn resolve_open(&mut self, path: &Path, snapshot: &BufferSnapshot);  // convert raw entries of a newly opened/reloaded file
    pub fn for_file(&self, path: &Path) -> Vec<Diagnostic>;                  // all servers, sorted by range start
    pub fn summary(&self) -> (usize, usize);                                 // (errors, warnings), project-wide incl. unopened files
    pub fn clear_server(&mut self, key: &ServerKey) -> Vec<PathBuf>;
    pub fn next(&self, from: (&Path, Point), forward: bool, scope: Scope) -> Option<(PathBuf, Diagnostic)>; // Scope::{File, Files}; skips Hint; files by path order; wraps
}
// Unopened files: entries kept raw (lsp Range) and converted with `locations::read_lines` when `next`/`for_file` needs Points (read the file synchronously — callers run these off the UI thread or on open files only; `next` across unopened files is called from a background task in Task 12).
// locations.rs
#[derive(Clone, Debug, PartialEq)] pub struct ResolvedLocation { pub path: Arc<Path>, pub range: Range<Point>, pub line_text: String }   // line_text = text of range.start.row
pub fn resolve_locations(locs: Vec<(PathBuf, lsp_types::Range)>, encoding: PositionEncoding, open: &dyn Fn(&Path) -> Option<BufferSnapshot>) -> Vec<ResolvedLocation>; // reads unopened files from disk (decode via lucerna_text::Buffer::load); unreadable files dropped; dedup; sorted by path then start
pub fn locations_from_response(resp: Option<lsp_types::GotoDefinitionResponse>) -> Vec<(PathBuf, lsp_types::Range)>; // LocationLink uses targetSelectionRange; non-file URIs dropped
```
- [ ] **Step 1: Failing tests:** sync — `open_once_per_key`, `change_bumps_version_full_and_incremental` (assert `received()` content-change shapes), `none_sync_sends_nothing`, `large_file_and_long_line_detected`, `watched_glob_relative_pattern_and_kind`; diagnostics — `stale_version_dropped` (Review Focus 1), `out_of_range_clamped`, `summary_counts_unopened`, `next_skips_hint_wraps_across_files`, `resolve_open_converts_raw_utf16` (emoji line, Review Focus 3), `clear_server_returns_paths`; locations — `location_link_uses_target_selection`, `unopened_file_read_from_disk`, `dedup_and_sort`.
- [ ] **Step 2–4:** FAIL → implement → PASS.
- [ ] **Step 5: Commit** `feat(lsp): document sync, watched files, diagnostics store, location resolution`.

## Task 7: Settings, theme colors, keybinding and setting defaults

**Files:** Modify `lucerna-settings/src/store.rs`, `assets/default-settings.json`, `assets/default-keybindings.json`; `lucerna-theme/src/{lib.rs,schema.rs}`; tests in `lucerna-settings/tests/` and `lucerna-theme` unit tests.

**Interfaces — Produces:**
```rust
impl SettingsStore { pub fn get_trusted<T: DeserializeOwned>(&self, key: &str, language: Option<&str>) -> Option<T>; } // `get` without the Workspace layer
// UiColors new fields: warning, info, hint, error_background, warning_background, info_background, hint_background,
//   error_border, warning_border, info_border, hint_border, link_text_hover, editor_highlighted_line_background, element_hover
```
Theme keys: `warning`, `info`, `hint`, `<x>.background`, `<x>.border`, `link_text.hover`, `editor.highlighted_line.background`, `element.hover`. Fallbacks (dark / light, RGBA hex): `error` keep existing; `warning` `#dec184ff`/`#a48819ff`; `info` `#74ade8ff`/`#5c78e2ff`; `hint` `#788ca6ff`/`#7274a7ff`; `error.background` `#d072771a`/`#fbdfd9ff`; `warning.background` `#dec1841a`/`#faf2e6ff`; `info.background` `#74ade81a`/`#e2e2faff`; `hint.background` `#5a6f891a`/`#e2e2faff`; `error.border` `#4c2b2cff`/`#f6c6bdff`; `warning.border` `#5d4c2fff`/`#f4e7d1ff`; `info.border` and `hint.border` `#293b5bff`/`#cbcdf6ff`; `link_text.hover` `#74ade8ff`/`#5c78e2ff`; `editor.highlighted_line.background` `#2f343eff`/`#ebebecff`; `element.hover` = existing `ghost_element_hover` fallback.
Default settings: every key of spec §8 table with its default, including `"[python]": { "lucerna.lsp.languageServers": ["basedpyright", "!pyright", "..."] }`. Keybindings: every row of spec §8 with `when` `editorTextFocus` for editor actions, `referencesViewFocus` for `references-view.next/prev`.

- [ ] **Step 1: Failing tests:** `get_trusted_ignores_workspace_layer` (workspace `lsp.x` set, user not → `None`; user set → user value; object merge across Default+User still works); `theme_reads_diagnostic_colors` (theme JSON with `"warning": "#112233ff"` → parsed; missing → fallback); `default_settings_have_phase3_keys` (each key resolves); `keymap_has_phase3_bindings` (`f12` → `editor.action.revealDefinition`, `cmd+t` → `workbench.action.showAllSymbols`, `ctrl+-` → `workbench.action.navigateBack`).
- [ ] **Step 2–4:** FAIL → implement → PASS (`cargo test -p lucerna-settings -p lucerna-theme`).
- [ ] **Step 5: Commit** `feat(settings,theme): Phase 3 settings, keybindings, trusted lookup and diagnostic colors`.

## Task 8: Document events and the `LanguageServers` bridge

**Files:** Modify `documents.rs` (events, `views_of`, `snapshot_of`, prompt `Closed`), `workbench.rs` (own `Entity<LanguageServers>`, forward worktree events, shutdown on release), `lib.rs`; create `language_servers.rs`; add `lucerna-lsp` (+ `test-support` in dev-deps) to `lucerna-workbench/Cargo.toml`; test `tests/lsp_sync.rs`, helpers `tests/support/lsp.rs`.

**Interfaces — Consumes:** Tasks 3–7. **Produces:**
```rust
// documents.rs
pub enum DocumentEvent {
    Opened { path: Arc<Path>, language_id: Option<Arc<str>>, snapshot: BufferSnapshot },
    Reloaded { path: Arc<Path>, snapshot: BufferSnapshot },
    Closed { path: Arc<Path> },
    LanguageChanged { path: Arc<Path>, language_id: Option<Arc<str>>, snapshot: BufferSnapshot },
}
impl EventEmitter<DocumentEvent> for Documents {}
impl Documents {
    pub fn views_of(&self, path: &Path) -> Vec<Entity<CodeView>>;
    pub fn snapshot_of(&self, path: &Path) -> Option<BufferSnapshot>;
    pub fn language_of(&self, path: &Path) -> Option<Arc<str>>;
    pub fn open_paths(&self) -> Vec<Arc<Path>>;
}
// `Closed` is emitted no later than the frame after the last view is dropped (Workbench calls `collect_closed` from `render` via `cx.defer`).
// language_servers.rs
pub enum SlotState { Starting, Running, Failed { message: String, stderr_tail: Vec<String> }, NotFound { message: String }, Stopped }
pub struct ServerStatus { pub key: ServerKey, pub state: SlotState, pub progress: Option<String> }
pub enum LanguageServersEvent { DiagnosticsChanged(Vec<Arc<Path>>), StatusChanged, ServerReady(ServerKey), DocumentChanged(Arc<Path>) }
pub struct LanguageServers;
impl EventEmitter<LanguageServersEvent> for LanguageServers {}
impl LanguageServers {
    pub fn new(root: PathBuf, documents: &Entity<Documents>, cx: &mut Context<Self>) -> Self;   // ProcessLauncher
    pub fn set_launcher(&mut self, launcher: Arc<dyn Launcher>);
    pub fn set_trusted(&mut self, trusted: bool, cx: &mut Context<Self>);   // false → stop all; true → open every open document
    pub fn is_trusted(&self) -> bool;
    pub fn statuses(&self) -> Vec<ServerStatus>;
    pub fn log(&self, server: &str) -> Vec<LogLine>;          // ring of 2000
    pub fn diagnostics(&self) -> &DiagnosticStore;
    pub fn running_for(&self, path: &Path) -> Vec<(ServerKey, Arc<LanguageServer>)>;
    pub fn wants_server(&self, path: &Path, language_id: &str, cx: &App) -> bool;   // a server is selected and enabled (used by trust modal)
    pub fn restart(&mut self, key: &ServerKey, cx: &mut Context<Self>);
    pub fn stop(&mut self, key: &ServerKey, cx: &mut Context<Self>);
    pub fn restart_all(&mut self, cx: &mut Context<Self>);
    pub fn stop_all(&mut self, cx: &mut Context<Self>);
    pub fn on_worktree_event(&mut self, ev: &WorktreeEvent, cx: &mut Context<Self>); // didChangeWatchedFiles, 100 ms batches
    pub fn shutdown_all(&mut self);                           // joins shutdown threads, ≤ 5 s total
}
impl Workbench { pub fn language_servers(&self) -> &Entity<LanguageServers>; }
```
Flow (spec §4, §6): subscribe to `DocumentEvent`; for `Opened` do nothing while untrusted (Task 9 flips `set_trusted`); otherwise read `lucerna.lsp.enabled` and `lucerna.lsp.languageServers` for the language and `lsp` with `get` (all layers), compute keys, start missing slots in `cx.background_spawn` (resolve_launch + launch) then `initialize` awaited via `cx.spawn`; queue `didOpen` while `Starting`. Each server's `ServerEvent` receiver is pumped by a `cx.spawn` loop. Diagnostics events go to `DiagnosticStore` (open snapshot + `DocumentSync::version`) and are batched 50 ms before emitting `DiagnosticsChanged` and calling `CodeView::set_diagnostics` (Task 11 adds that method; until then store only). Settings changes: observe `AppState` global; on change rebuild `ServersSettings` and apply `SettingsDiff`. `Exited` → `Failed { message: "exited with code N" }`, `clear_server`, emit `StatusChanged`. Workbench `on_release` (window close) and app quit → `shutdown_all`.

Test helper `tests/support/lsp.rs`: `fn lsp_workbench(root, fake: FakeLauncher, cx) -> (Entity<Workbench>, &mut VisualTestContext)` that inits AppState with `session.trustAllWorktrees: true` in the user layer and installs the launcher.

- [ ] **Step 1: Failing tests** (`tests/lsp_sync.rs`): `opening_rust_file_starts_server_and_sends_did_open` (FakeLauncher with `rust-analyzer`; `received()` contains `textDocument/didOpen` with `languageId:"rust"`, `version:0`); `two_tabs_one_did_open_close_after_last` (Review Focus 4); `reload_sends_did_change_full_text`; `untouched_reload_sends_nothing`; `missing_binary_is_not_found_status` (message exact); `crash_marks_failed_and_clears_diagnostics`; `deleted_file_sends_did_close`; `language_change_reopens_with_new_server`; `disabled_by_setting_starts_nothing`; `settings_change_restarts_on_binary_change`; `watched_file_event_forwarded`.
- [ ] **Step 2–4:** FAIL → implement → PASS (`cargo test -p lucerna-workbench --test lsp_sync`, then the whole workbench suite stays green).
- [ ] **Step 5: Commit** `feat(workbench): LanguageServers bridge with document sync`.

## Task 9: Folder trust

**Files:** Create `trust.rs`; modify `layout/title_bar.rs`, `workbench.rs`, `commands` registration, `icons.rs` (Lucide `triangle-alert` if absent); test `tests/trust.rs`.

**Interfaces — Produces:**
```rust
pub struct TrustStore { /* path: <data_dir>/trusted.json */ }
impl TrustStore {
    pub fn load(data_dir: &Path) -> TrustStore;        // corrupt → empty
    pub fn is_trusted(&self, folder: &Path) -> bool;   // folders ∪ descendants of parents
    pub fn trust(&mut self, folder: &Path, parent: bool) -> io::Result<()>;  // parent=true stores folder.parent()
    pub fn clear(&mut self) -> io::Result<()>;
}
pub struct TrustModal;  // gpui view; events TrustModalEvent::{Trust { parent: bool }, Stay}
```
Copy and layout exactly as spec §5. Behaviour: `session.trustAllWorktrees` (read with `get_trusted`, i.e. User/Default only) short-circuits to trusted without writing. The modal auto-opens once per session on the first `DocumentEvent::Opened` for which `LanguageServers::wants_server` is true while untrusted. "Stay" suppresses auto-open for the session. Title bar shows the "Restricted Mode" button (warning icon + label, `ghost_element.hover` on hover) whenever untrusted; it opens the modal. Commands `workbench.trust.manage` (open modal), `workbench.trust.clear` (clear store, `set_trusted(false)` on this window). Single-file mode uses the file's parent folder.

- [ ] **Step 1: Failing tests:** `trust_store_parent_covers_children`, `corrupt_trust_file_is_empty`, `untrusted_folder_spawns_nothing` (FakeLauncher `launches()` empty after opening a .rs file; title bar debug selector `restricted-mode` present), `modal_opens_on_first_server_file_and_trust_starts_server` (click "Trust and Continue" → `didOpen` received; `trusted.json` written), `stay_does_not_reopen`, `workspace_lsp_binary_path_ignored_until_trusted` (Review Focus 5: workspace `.lucerna/settings.json` with `lsp.rust-analyzer.binary.path = /bin/false-path`; before trust no launch; after trust the launch spec uses that path), `trust_all_worktrees_setting`.
- [ ] **Step 2–4:** FAIL → implement → PASS.
- [ ] **Step 5: Commit** `feat(workbench): Zed-style folder trust for language servers`.

## Task 10: Server status button, progress text, log tab

**Files:** Create `lsp_status.rs`; modify `editor_groups/model.rs` (`TabItem::LspLog(Arc<str>)` — not saved in workspace state, singleton per server name), `editor_groups/view.rs`/tab body factory, `workbench.rs` (status item registration, commands), `icons.rs` (`zap`); test `tests/lsp_status.rs`.

**Interfaces — Consumes:** Task 8 `statuses()`, `log()`, `restart/stop*`. **Produces:** commands `lucerna.lsp.restart`, `lucerna.lsp.restartAll`, `lucerna.lsp.stopAll`, `lucerna.lsp.showLog`; debug selectors `lsp-button`, `lsp-popover`, `lsp-progress`, `lsp-log`.

UI per spec §7.7 (button via `icon_button`, dot 6 px, popover via `popover` anchored above the status bar on the right, rows with dot + name + muted root + text buttons). Progress text: newest `Progress` not `done`, format `"{server}: {title} {percentage}%"` + `" — {message}"` if present, truncated at 60 chars, `" +N"` for others; `"Starting {server}…"` while starting. The log tab body is a read-only list (`uniform_list`, code font) that follows the end while scrolled to the end; ⌘A then ⌘C copies all lines. A `ShowMessage` of kind Error is also shown as a row under its server in the popover until the next restart.

- [ ] **Step 1: Failing tests:** `button_hidden_without_servers`, `not_found_shows_warning_dot_and_message`, `failed_shows_stderr_tail`, `restart_relaunches` (`launches().len() == 2`), `progress_text_format`, `show_log_opens_tab_with_stderr_lines`.
- [ ] **Step 2–4:** FAIL → implement → PASS.
- [ ] **Step 5: Commit** `feat(workbench): language server status button, progress and log tab`.

## Task 11: Diagnostics in the code view

**Files:** Create `code_view/diagnostics.rs`; modify `code_view/mod.rs` (`set_diagnostics`), `code_view/element.rs` (frame data, gutter width), `code_view/element/paint.rs`, `icons.rs` (`circle-x`, `triangle-alert`, `info`); test `tests/diagnostics.rs` + unit tests in `code_view/diagnostics.rs`.

**Interfaces — Consumes:** Task 6 `Diagnostic`, Task 7 colors. **Produces:**
```rust
impl CodeView {
    pub fn set_diagnostics(&mut self, diagnostics: Arc<[Diagnostic]>, cx: &mut Context<Self>);
    pub fn diagnostics(&self) -> &[Diagnostic];
    pub fn diagnostic_rows(&self) -> &DiagnosticRows;   // for tests
}
// code_view/diagnostics.rs (no gpui types)
pub struct DiagnosticRows { /* built once per set_diagnostics / reload */ }
impl DiagnosticRows {
    pub fn build(diags: &[Diagnostic], snapshot: &BufferSnapshot) -> DiagnosticRows;
    pub fn marks_in(&self, rows: Range<u32>) -> impl Iterator<Item = &Mark>;  // Mark { range: Range<Point>, severity, style: MarkStyle::{Wavy, Fade, Strike} }
    pub fn gutter(&self, row: u32) -> Option<Severity>;        // highest of Error/Warning/Information on the row; Hint → None
    pub fn inline(&self, row: u32) -> Option<(Severity, &str)>; // first line of the most severe non-hint message
    pub fn scrollbar(&self) -> &[(u32, Severity)];             // rows with Error or Warning
}
```
Rules (spec §7.1): empty range → one char (or the last char of the line, or nothing on an empty line); `unnecessary` with severity below Warning → `Fade` instead of `Wavy`; `deprecated` → `Strike` (plus wavy if Error/Warning). Painting: wavy 1 px underline via gpui `UnderlineStyle { wavy: true, thickness: px(1.) }` along `range_bounds`; `Fade` = an `editor.background` quad at 70% alpha over the glyphs (no reshaping); `Strike` = a 1 px line at mid-height in the text color. Gutter: 16 px column at the gutter's left edge when `lucerna.diagnostics.gutterIcons` is true and the view has any diagnostics source (a server is running for the file — `set_diagnostics` is called even with an empty slice to switch the column on); icons 12 px, vertically centred. Inline: when `lucerna.diagnostics.inline.enabled`, after the row's last piece, 4 em gap, pill as spec; clipped at the text bounds. Scrollbar marks: 2 px tall, right half of the scrollbar track, Error and Warning only. Wire `LanguageServers` (Task 8) to call `set_diagnostics` for every view of each changed path, and on `Opened`/`Reloaded` (after `resolve_open`).

- [ ] **Step 1: Failing tests:** unit — `empty_range_widens_to_one_char`, `unnecessary_hint_fades`, `gutter_takes_highest_severity_and_skips_hint`, `inline_uses_first_line`, `emoji_line_mark_columns` (Review Focus 3: diagnostic on `"let 😀 = x"` UTF-16 col 6..7 resolves to the byte range of `=`); gpui — `diagnostics_reach_view_after_publish` (fake pushes `publishDiagnostics` after `didOpen`; `view.diagnostics().len() == 2` after 50 ms advance), `gutter_column_only_when_enabled` (settings off → gutter width unchanged), `inline_off_by_default`, `stale_version_ignored` (Review Focus 1: fake publishes with version 0 after a reload → not shown).
- [ ] **Step 2–4:** FAIL → implement → PASS.
- [ ] **Step 5: Commit** `feat(workbench): render diagnostics (squiggles, gutter, inline, scrollbar)`.

## Task 12: Navigation history

**Files:** Create `nav_history.rs`; modify `workbench.rs` (`open_path_at` and the in-file jump helper record history), `commands`; test `tests/nav_history.rs` + unit tests.

**Interfaces — Produces:**
```rust
#[derive(Clone, Debug, PartialEq)] pub struct NavEntry { pub path: Arc<Path>, pub point: Point, pub scroll_top_row: u32 }
pub struct NavHistory;  // per group index
impl NavHistory {
    pub fn record(&mut self, from: NavEntry, to: &NavEntry);   // skip when same path and |row diff| < 10; clears forward; cap 50
    pub fn back(&mut self, current: NavEntry) -> Option<NavEntry>;
    pub fn forward(&mut self, current: NavEntry) -> Option<NavEntry>;
}
impl Workbench {
    pub fn jump_to(&mut self, path: Arc<Path>, point: Point, mode: OpenMode, cx: &mut Context<Self>);  // records history, then open_path_at; same-file jumps go through here too
}
```
Commands `workbench.action.navigateBack` / `navigateForward` (open non-preview, restore cursor + scroll, skip entries whose file no longer exists). Existing callers that jump (Quick Open file / `:line`, Search results) switch to `jump_to`.

- [ ] **Step 1: Failing tests:** unit `record_skips_small_same_file_moves`, `record_clears_forward`, `cap_50`; gpui `back_and_forward_across_files` (Quick Open jump a.rs:1 → b.rs:40, `ctrl+-` returns to a.rs row 0, `ctrl+shift+-` to b.rs row 39), `back_skips_deleted_file`.
- [ ] **Step 2–4:** FAIL → implement → PASS.
- [ ] **Step 5: Commit** `feat(workbench): go back / forward navigation history`.

## Task 13: Diagnostics in the status bar and F8 navigation

**Files:** Create `diagnostics_status.rs`; modify `workbench.rs` (status item, commands); test add to `tests/diagnostics.rs`.

**Interfaces — Consumes:** Task 6 `DiagnosticStore::{summary,next}`, Task 12 `jump_to`. **Produces:** commands `editor.action.marker.nextInFiles`, `prevInFiles`, `next`, `prev`; debug selectors `diagnostics-counts`, `diagnostics-message`. After a jump, hover is opened immediately at the point (Task 15 provides `HoverController::show_now`; until then, jump only — Task 15 adds the call).

Behaviour per spec §7.1 Status bar + F8. `next` for `Files` scope runs off the UI thread (`cx.background_spawn`) because unopened files may need reading.

- [ ] **Step 1: Failing tests:** `counts_update_after_publish` (text `"2"` and `"1"` under `diagnostics-counts`), `check_icon_when_clean`, `hidden_before_any_server`, `message_under_cursor`, `f8_moves_across_files_and_wraps`, `alt_f8_stays_in_file`, `hints_skipped`, `click_counts_runs_next_in_files`.
- [ ] **Step 2–4:** FAIL → implement → PASS.
- [ ] **Step 5: Commit** `feat(workbench): diagnostics status bar item and marker navigation`.

## Task 14: Markdown component

**Files:** Create `markdown.rs`; add `pulldown-cmark` to workbench deps; unit tests in the module, gpui render test in `tests/hover.rs`.

**Interfaces — Produces:**
```rust
pub struct MarkdownBlocks(Vec<Block>);     // parsed once, rendered many times
pub fn parse_markdown(source: &str, fallback_language: Option<&str>) -> MarkdownBlocks;   // truncates at 100_000 bytes, closing an open fence
pub enum Block { Paragraph(Vec<Inline>), Heading(u8, Vec<Inline>), Code { language: Option<String>, text: String }, List { ordered: bool, items: Vec<Vec<Block>> }, Quote(Vec<Block>), Rule }
pub enum Inline { Text(String), Strong(Vec<Inline>), Emph(Vec<Inline>), Code(String), Link { url: String, children: Vec<Inline> }, SoftBreak, HardBreak }
pub fn render_markdown(blocks: &MarkdownBlocks, cx: &App) -> AnyElement;   // styles per spec §7.2; code blocks highlighted through lucerna-syntax highlights for `language_by_name`/id, else fallback language
pub fn open_markdown_link(url: &str, wb: &mut Workbench, cx: &mut Context<Workbench>);  // http(s) → platform open; file://path#L10 → jump_to
```
Highlighting a code block: build a `Buffer` from the text, `SyntaxState::new` + parse synchronously within `SYNC_PARSE_BUDGET`; on timeout render plain. Cache per `MarkdownBlocks`.

- [ ] **Step 1: Failing tests:** `parses_fenced_code_with_language`, `marked_string_language_fallback`, `truncation_closes_fence`, `nested_list`, `link_inline`; gpui `renders_without_panic_and_highlights_rust_block` (render into a test window; assert a `StyledText` with >1 run exists via debug selector `md-code-0`).
- [ ] **Step 2–4:** FAIL → implement → PASS; `cargo deny check`.
- [ ] **Step 5: Commit** `feat(workbench): markdown rendering component`.

## Task 15: Hover

**Files:** Create `hover.rs`; modify `code_view/element.rs` (mouse-move → hover controller, hovered-range background), `code_view/mod.rs`, `language_servers.rs` (`hover` request), `diagnostics_status.rs` (F8 → `show_now`), commands; test `tests/hover.rs`.

**Interfaces — Produces:**
```rust
impl LanguageServers {
    pub fn hover(&self, path: &Path, point: Point, cx: &mut Context<Self>) -> Task<Option<HoverInfo>>;   // first server with hoverProvider; 10 s timeout; drop cancels
}
pub struct HoverInfo { pub markdown: String, pub range: Option<Range<Point>>, pub language: Option<String> }
pub struct HoverController;   // owned by CodeView (one per view)
impl HoverController {
    pub fn mouse_moved(&mut self, point: Option<Point>, mouse: gpui::Point<Pixels>, cx: &mut Context<CodeView>);
    pub fn show_now(&mut self, point: Point, cx: &mut Context<CodeView>);   // editor.action.showHover and F8
    pub fn hide(&mut self, cx: &mut Context<CodeView>);
    pub fn is_visible(&self) -> bool;
}
```
Timing per spec §7.2 with gpui timers (`cx.background_executor().timer`): request at `delay/2`, show at `delay` or on arrival. Diagnostic block: diagnostics of the view whose range contains the point (narrowest first), each with message and a muted `source(code)` line where `code` links to `code_href`. Popover placement and size per spec (above if fits, else below; 8 px from right edge; width min(half editor, 120 ch) ≥ 20 ch; height min(half editor, 16 lines), scrollable). Context key `lspHoverVisible`. Esc, scroll, focus loss hide.

- [ ] **Step 1: Failing tests:** `hover_appears_after_delay` (advance 149 ms → no request; 151 ms → `textDocument/hover` received; 300 ms → popover `hover-popover` visible with text), `moving_away_cancels_request` (`$/cancelRequest` received), `sticky_keeps_open_when_entering_popover`, `hides_after_hiding_delay`, `show_hover_command_no_delay`, `diagnostic_block_above_info`, `disabled_by_setting`.
- [ ] **Step 2–4:** FAIL → implement → PASS.
- [ ] **Step 5: Commit** `feat(workbench): LSP hover with diagnostic block`.

## Task 16: Go to definition commands

**Files:** Create `goto.rs`; modify `language_servers.rs` (`locations` request), commands registration, `icons`; test `tests/goto.rs`.

**Interfaces — Produces:**
```rust
pub enum GotoKind { Definition, TypeDefinition, Implementation, Declaration, References }
impl LanguageServers {
    pub fn locations(&self, path: &Path, point: Point, kind: GotoKind, cx: &mut Context<Self>) -> Task<Vec<ResolvedLocation>>; // all capable servers, joined, deduped; References uses includeDeclaration:true; resolution off UI thread
}
pub enum GotoMode { Reveal, Peek, Aside }
pub fn run_goto(wb: &mut Workbench, kind: GotoKind, mode: GotoMode, window: &mut Window, cx: &mut Context<Workbench>);
pub fn show_no_result(view: &Entity<CodeView>, kind: GotoKind, word: &str, cx: &mut App);  // "No definition found for 'word'" etc., 3 s
```
Decision table per spec §7.3 incl. `editor.gotoLocation.multiple*` (`peek` | `gotoAndPeek` | `goto`), "same location → references" fallback (only for non-peek Definition), no-result popover copy (Definition/Type/Implementation/Declaration/References variants), `Aside` opens in the next group (create a split if only one). The picker is Task 18; until then multi-result opens the first location and logs — Task 18 replaces that call with `LocationsPicker::open`.

Commands registered: all of spec §7.3 table plus `editor.action.goToReferences`.

- [ ] **Step 1: Failing tests:** `f12_single_result_jumps` (fake `textDocument/definition` → one Location in b.rs; active tab b.rs, cursor at range start, nav history has an entry), `location_link_target_selection_used`, `same_location_falls_back_to_references` (`textDocument/references` received), `no_result_popover_text`, `aside_opens_in_second_group`, `goto_setting_goes_to_first`.
- [ ] **Step 2–4:** FAIL → implement → PASS.
- [ ] **Step 5: Commit** `feat(workbench): go to definition / type / implementation / declaration`.

## Task 17: ⌘-hover link and ⌘-click

**Files:** Create `code_view/link.rs`; modify `code_view/element.rs` (modifiers-changed and mouse-move handling, cursor style, click), `code_view/cache.rs` `ShapeKey` (include link range of the row so the link text is recolored); test add to `tests/goto.rs`.

**Interfaces — Consumes:** Task 16 `LanguageServers::locations`, `run_goto`. **Produces:** `CodeView::link_range(&self) -> Option<Range<Point>>` (for tests). Platform modifier: `cmd` on macOS, `ctrl` elsewhere (`Modifiers::secondary()`).

- [ ] **Step 1: Failing tests:** `cmd_hover_underlines_origin_range` (`link_range()` equals `originSelectionRange` after moving with ⌘ held), `cmd_click_jumps`, `alt_cmd_click_opens_aside`, `release_cmd_clears_link`, `cached_by_range_no_second_request` (two moves within the range → one `textDocument/definition`).
- [ ] **Step 2–4:** FAIL → implement → PASS.
- [ ] **Step 5: Commit** `feat(workbench): cmd-hover definition link and cmd-click`.

## Task 18: Locations picker with preview

**Files:** Create `locations_picker.rs`; modify `goto.rs` (use the picker), `quick_open/picker.rs` only to export what is needed; test `tests/goto.rs`.

**Interfaces — Produces:**
```rust
pub struct LocationsPicker;   // gpui view
impl LocationsPicker {
    pub fn open(wb: &mut Workbench, kind: GotoKind, locations: Vec<ResolvedLocation>, current: Arc<Path>, window: &mut Window, cx: &mut Context<Workbench>);
    pub fn selected(&self) -> Option<&ResolvedLocation>;
    pub fn row_texts(&self) -> Vec<String>;   // for tests: "header:<file>" / "<line>:<text>"
}
```
Layout, sizes, placeholder copy, sort order (current file first), filter, keys (↑/↓ skip headers, Enter preview-open, ⌘Enter aside, Esc), click / double-click per spec §7.4. Preview: `Documents::open(path)` to get a `CodeView`, `go_to` + highlighted rows (`CodeView::set_line_highlights(Vec<Range<u32>>)` — new small API painting `editor.highlighted_line.background`, also used by Task 21); dropping the picker drops the view.

- [ ] **Step 1: Failing tests:** `multiple_definitions_open_picker_grouped`, `filter_narrows_rows`, `arrow_moves_preview` (preview view path changes), `enter_opens_selected`, `cmd_enter_opens_aside`, `esc_closes_without_jump`, `peek_with_single_result_still_opens_picker`.
- [ ] **Step 2–4:** FAIL → implement → PASS.
- [ ] **Step 5: Commit** `feat(workbench): locations picker with preview`.

## Task 19: Extract `excerpt_list` from the Search tab

**Files:** Create `excerpt_list/{mod.rs,model.rs,highlight.rs,render.rs}`; move code out of `search_view/{model.rs,highlight.rs,view.rs}`; modify `search_view/*` to use it. No behaviour change.

**Interfaces — Produces:**
```rust
pub struct ExcerptList;     // gpui view: rows (Header/Line/Separator), collapse, keyboard selection, background highlighting of visible files, open-on-activate
impl ExcerptList {
    pub fn new(cx: &mut Context<Self>) -> Self;
    pub fn set_files(&mut self, files: Vec<(Arc<Path>, Vec<lucerna_project::Excerpt>)>, cx: &mut Context<Self>);
    pub fn add_files(&mut self, batch: Vec<(Arc<Path>, Vec<Excerpt>)>, cx: &mut Context<Self>);   // streaming (Search)
    pub fn clear(&mut self, cx: &mut Context<Self>);
    pub fn results(&self) -> &Results;
    pub fn selected(&self) -> Option<LineRef>;
    pub fn step_match(&mut self, forward: bool, cx: &mut Context<Self>) -> Option<LineRef>;
    pub fn toggle_collapsed(&mut self, file: usize, cx: &mut Context<Self>);
    pub fn set_all_collapsed(&mut self, collapsed: bool, cx: &mut Context<Self>);
    pub fn focus_handle(&self) -> &FocusHandle;
}
pub enum ExcerptListEvent { Activate(LineRef) }
```
Names inside (`Results`, `Row`, `LineRef`, `LineAnchor`, `FileResult`) move unchanged.

- [ ] **Step 1:** Run `cargo test -p lucerna-workbench --test search_view` and the unit tests in `search_view` → record PASS baseline.
- [ ] **Step 2:** Move code; keep every Search test passing unchanged (only `use` paths in tests may change).
- [ ] **Step 3:** `cargo test -p lucerna-workbench` → PASS; `cargo clippy -p lucerna-workbench --all-targets -- -D warnings` → clean.
- [ ] **Step 4: Commit** `refactor(workbench): extract excerpt_list from the Search tab`.

## Task 20: References tab and `goToReferences`

**Files:** Create `references_view.rs`; modify `editor_groups/model.rs` (`TabItem::References`, singleton, not persisted), tab body factory, `goto.rs` (References kind uses picker / tab), `keys.rs` (`referencesViewFocus`), commands; test `tests/references.rs`.

**Interfaces — Consumes:** Task 16 `locations(.., References)`, Task 19 `ExcerptList`. **Produces:** `pub fn find_references_in_tab(wb, window, cx)`; commands `references-view.findReferences`, `references-view.next`, `references-view.prev`; debug selectors `references-summary`.

Excerpts built off the UI thread: for each file, read lines, one line of context above/below each reference, merge overlapping/adjacent, `LineMatch.ranges` = the reference byte ranges on that line. Summary copy: `"{n} references in {m} files"` (singular forms `1 reference`, `1 file`); tab title `References to {word}`; refresh button re-runs at the stored path/point.

- [ ] **Step 1: Failing tests:** `shift_f12_multiple_opens_picker`, `shift_alt_f12_opens_single_tab_reused` (run twice → one tab, title updated), `summary_and_headers`, `enter_opens_reference`, `f4_steps_references`, `search_tab_unaffected` (open both tabs; Search still searches).
- [ ] **Step 2–4:** FAIL → implement → PASS.
- [ ] **Step 5: Commit** `feat(workbench): references tab and peek references`.

## Task 21: Symbol types, `outline.scm`, symbol source, Quick Open `@`

**Files:** Create `lucerna-syntax/src/{symbols.rs,outline.rs}`, `lucerna-lsp/src/symbols.rs`, `workbench/src/symbols/{mod.rs,source.rs,quick_open.rs}`; modify `lucerna-syntax/src/{lib.rs,registry.rs}`, `quick_open/{query.rs,view.rs,mod.rs}`, `language_servers.rs` (`document_symbols`); tests `lucerna-syntax/tests/outline.rs`, `lucerna-lsp/tests/symbols.rs`, `workbench/tests/symbols.rs`, `tests/quick_open_query.rs`.

**Interfaces — Produces:**
```rust
// lucerna-syntax
pub enum SymbolKind { File, Module, Namespace, Package, Class, Method, Property, Field, Constructor, Enum, Interface, Function, Variable, Constant, String, Number, Boolean, Array, Object, Key, Null, EnumMember, Struct, Event, Operator, TypeParameter }  // LSP order
pub struct Symbol { pub name: String, pub detail: Option<String>, pub kind: SymbolKind, pub range: Range<Point>, pub selection_range: Range<Point>, pub children: Vec<Symbol> }
pub struct SymbolTree(pub Vec<Symbol>);
impl SymbolTree {
    pub fn path_at(&self, p: Point) -> Vec<&Symbol>;
    pub fn flatten(&self) -> Vec<FlatSymbol<'_>>;      // FlatSymbol { depth: usize, ancestors: Vec<&str>, symbol: &Symbol }, document order
    pub fn from_flat(items: Vec<Symbol>) -> SymbolTree; // nest by range containment
}
impl Language { pub fn has_outline(&self) -> bool; }
impl SyntaxState { pub fn outline(&self, snapshot: &BufferSnapshot) -> Option<SymbolTree>; }   // §3.2 rules incl. kind guessing
// lucerna-lsp
pub fn symbols_from_response(resp: lsp_types::DocumentSymbolResponse, snapshot: &BufferSnapshot, enc: PositionEncoding) -> SymbolTree;
// workbench
impl LanguageServers { pub fn document_symbols(&self, path: &Path, cx: &mut Context<Self>) -> Task<Option<SymbolTree>>; }
pub struct SymbolSource;  // per Workbench: cache (path, revision) → Option<SymbolTree>; 300 ms debounce; empty LSP result cached 2 s; refreshes on Opened/Reloaded/ServerReady; falls back to SyntaxState::outline
impl SymbolSource { pub fn get(&mut self, path: &Path, cx: &mut Context<Workbench>) -> SymbolState; } // SymbolState::{Ready(Arc<SymbolTree>), Loading, Unavailable}
// quick_open/query.rs
QuickOpenQuery::Symbols { text: String, group_by_kind: bool }        // "@" / "@:"
QuickOpenQuery::WorkspaceSymbols(String)                             // "#" (Task 23 renders it)
```
`@` behaviour per spec §7.6 (rows, indentation, ancestor suffix, single-word match must fall in the name, `@:` group headers `"{kind plural} ({n})"`, initial selection = deepest symbol containing cursor, preview with `CodeView::set_line_highlights` + scroll, Enter records history via `jump_to`, Esc restores cursor and scroll, empty-state copy). Kind icons: Lucide set chosen in `icons.rs` (one per kind group: function/method/constructor → `square-function`; class/struct → `box`; interface → `component`; enum/enum member → `list`; field/property → `brackets`; variable → `variable`; constant → `hash`; module/namespace/package/file → `package`; type parameter → `type`; others → `circle`). Command `workbench.action.gotoSymbol` opens Quick Open with `@`.

- [ ] **Step 1: Failing tests:** syntax — `outline_nests_by_range_and_guesses_kind` (fixture `outline.scm` against the bundled Rust grammar in the test: `(function_item "fn" @context name: (_) @name) @item`, `(impl_item "impl" @context type: (_) @name) @item`); lsp — `flat_symbol_information_nests`, `hierarchical_kept`, `utf16_selection_range`; query — `at_prefix_parses`, `at_colon_groups`; workbench — `at_lists_lsp_symbols_indented`, `at_falls_back_to_outline_without_server` (extension test language with `outline.scm`), `filter_matches_in_name_only_for_single_word`, `group_by_kind_headers`, `preview_scrolls_and_esc_restores`, `enter_jumps_and_records_history`, `no_source_message`.
- [ ] **Step 2–4:** FAIL → implement → PASS.
- [ ] **Step 5: Commit** `feat: document symbols from LSP or outline.scm, Quick Open @`.

## Task 22: Breadcrumbs

**Files:** Create `symbols/breadcrumbs.rs`; modify `editor_groups/view.rs` (the existing path bar becomes the breadcrumbs row: same 32 px row, `PATH_BAR_HEIGHT`), commands (`breadcrumbs.focusAndSelect`); test add to `tests/symbols.rs`.

**Interfaces — Consumes:** Task 21 `SymbolSource`, `SymbolTree::path_at`. **Produces:** `pub fn breadcrumb_segments(path_text: &str, symbols: &[&Symbol], file_path: FilePathMode, symbol_path: SymbolPathMode) -> Vec<Segment>` (pure; `Segment::{Dir(String), File(String), Symbol { kind, name }, Ellipsis}`; >12 → first 6 + `Ellipsis` + last 6). With `breadcrumbs.enabled: false` the row shows the current plain path text exactly as today. Only file tabs get symbols; the row is a button opening Quick Open `@`.

- [ ] **Step 1: Failing tests:** unit `segments_split_dirs_and_symbols`, `file_path_last_only`, `symbol_path_off`, `more_than_12_collapses`; gpui `breadcrumbs_follow_cursor` (cursor in `impl Foo { fn bar` → row text contains `Foo` and `bar`), `click_opens_symbol_quick_open`, `disabled_shows_plain_path`.
- [ ] **Step 2–4:** FAIL → implement → PASS.
- [ ] **Step 5: Commit** `feat(workbench): breadcrumbs with symbol path`.

## Task 23: Quick Open `#` workspace symbols

**Files:** Create `symbols/workspace.rs`; modify `quick_open/view.rs`, `language_servers.rs` (`workspace_symbols`), commands; test add to `tests/symbols.rs`.

**Interfaces — Produces:**
```rust
pub struct WorkspaceSymbolItem { pub name: String, pub kind: SymbolKind, pub container: Option<String>, pub path: Arc<Path>, pub range: Option<Range<Point>>, pub in_project: bool }
impl LanguageServers {
    pub fn workspace_symbols(&self, query: String, cx: &mut Context<Self>) -> Task<Vec<WorkspaceSymbolItem>>;   // all running servers with workspaceSymbolProvider; resolves `workspaceSymbol/resolve` lazily on confirm when range missing
}
```
Behaviour per spec §7.6 `#`: 100 ms debounce, drop previous task (cancel), client fuzzy re-filter with nucleo, in-project (under root and not ignored per the worktree snapshot) first, max 100, two-line rows, Enter / ⌘Enter, no preview, copy for empty query / no server. Command `workbench.action.showAllSymbols` opens Quick Open with `#`.

- [ ] **Step 1: Failing tests:** `hash_debounces_and_cancels` (type 3 chars within 100 ms → one `workspace/symbol`; type again → `$/cancelRequest` for the old id if still pending), `project_results_first_max_100`, `enter_opens_symbol`, `no_server_message`, `empty_query_message`.
- [ ] **Step 2–4:** FAIL → implement → PASS.
- [ ] **Step 5: Commit** `feat(workbench): Quick Open # workspace symbols`.

## Task 24: Vim bindings

**Files:** Modify `lucerna-vim/src/{lib.rs,state.rs}`, `workbench/src/vim.rs`; tests in `lucerna-vim` and `workbench/tests/vim.rs`.

**Interfaces — Produces:** `Action::RunCommand(&'static str)`. Normal mode: `gd` → `editor.action.revealDefinition`, `gD` → `editor.action.revealDeclaration`, `gh` → `editor.action.showHover`, `]d` → `editor.action.marker.next`, `[d` → `editor.action.marker.prev`, `ctrl-o` → `workbench.action.navigateBack`, `ctrl-i` → `workbench.action.navigateForward`. The workbench runs the command through `CommandRegistry::run`.

- [ ] **Step 1: Failing tests:** vim unit `gd_runs_reveal_definition`, `bracket_d_runs_marker_next`, `ctrl_o_runs_navigate_back`; workbench `vim_gd_jumps_to_definition` (fake server).
- [ ] **Step 2–4:** FAIL → implement → PASS.
- [ ] **Step 5: Commit** `feat(vim): LSP and navigation bindings`.

## Task 25: Docs, real-server check, UI review, bundle

**Files:** Modify `docs/superpowers/specs/2026-11-08-lucerna-phase1-design.md` §1.3 (row 3b), `docs/superpowers/specs/2026-11-09-lucerna-phase2-extensions-design.md` D1, `extensions/page.rs` copy `Language server: Phase 3b`, spec status line; README feature list if it lists phases; `lucerna-lsp/tests/rust_analyzer.rs` (`#[ignore]`).

- [ ] **Step 1:** Write `rust_analyzer_definition_and_hover` (`#[ignore]`): small fixture crate in a tempdir, real `rust-analyzer` from PATH, wait for `$/progress` end or 60 s, `definition` on a call resolves to the fn, `hover` contains its signature. Run `cargo test -p lucerna-lsp -- --ignored rust_analyzer` → PASS (record output; if rust-analyzer is missing, report instead of claiming).
- [ ] **Step 2:** Doc/copy updates per spec §14; page test expecting `Phase 3` updated.
- [ ] **Step 3:** Full verification: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo deny check` → all clean.
- [ ] **Step 4:** UI review against Zed: run the app on the Lucerna repo with rust-analyzer; screenshots of hover, squiggles + gutter + inline, picker, References tab, breadcrumbs, `@`, `#`, ⚡ popover, trust modal saved to `docs/superpowers/plans/phase3-screenshots/`; fix spacing/color deviations found.
- [ ] **Step 5:** `scripts/bundle-app.sh --install` → succeeds; launch the installed app once.
- [ ] **Step 6: Commit** `docs: Phase 3 status, Phase 3b roadmap entry` (+ any UI fixes as separate commits).
