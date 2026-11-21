# Phase 3: understanding code through LSP

Date 2026-11-10. Status: implemented (commits 3a9951d..HEAD).

## 1. Goals

Lucerna reads code like an IDE. Users jump to definitions, see references, hover for types and documentation, see compiler errors right in the file, and find symbols in a file or the whole project, without opening VS Code. Lucerna is still read-only (Phase 4 added editing, see `2026-11-11-lucerna-phase4-editing-design.md`): completion, rename, format and code actions belong to Phase 4.

Decided with the user:

| # | Decision | Reason |
|---|---|---|
| D1 | **Servers come from PATH through built-in adapters**: rust-analyzer, gopls, pyright / basedpyright, typescript-language-server, clangd. No automatic server download. | These are the main languages. Zed ships built-in servers for them in Rust (GPL), not through extensions. |
| D2 | **The WASM host for Zed extensions is deferred to Phase 3b**, with its own spec. | A WASM host does not bring Rust/Go/Python/TS, and it needs `cargo` + `wasm32-wasip2` on the user's machine to build `extension.wasm` (Lucerna does not use `api.zed.dev`). The host alone is estimated at 2–4 weeks. |
| D3 | **Let users declare servers in settings** (`lsp.<name>.command`…), following Helix. Differs from VS Code and Zed. | Without a WASM host, this is the cheapest way to get LSP for every language; the risk of running arbitrary commands is blocked by the folder trust mechanism (Workspace settings are read only once trusted). See `2026-11-11-lsp-server-table-design.md`. |
| D4 | **The `lucerna-lsp` crate does not use gpui**, has its own transport, and takes data types from `lsp-types`. | Testable with a fake server; does not pull in tokio. |
| D5 | **Multiple results and peek open a picker with a preview** (following Zed's `lsp_locations`); `⇧⌥F12` opens a **References tab** that shares its UI with the Search tab. No inline peek area in the editor. | UI follows Zed. |
| D6 | **Symbols come from LSP `documentSymbol`**; with no server, use the language extension's `outline.scm` (if any). | As accurate as VS Code, and an outline still exists without a server. |
| D7 | **The outline has only the `@` modal and breadcrumbs**, no panel. | Minimal. |
| D8 | **Diagnostics:** squiggles (always shown even though read-only), gutter icons, inline messages (off by default), scrollbar marks, status bar, F8. No Problems tab, no Explorer coloring. | The user's choice. |
| D9 | **Folder trust like Zed**: a new folder is in Restricted Mode, and no server runs until the user trusts it. | Servers run the project's code (build.rs, proc-macros, `go list`). |
| D10 | A crashed server is **not restarted automatically** (like Zed); the user clicks Restart. | Errors stay visible, with no crash loop. |

**Success means:**
- Open a trusted Rust crate with `rust-analyzer` on PATH: once indexing is done, F12 on a function jumps to its definition; ⌘-click does too; ⇧F12 shows a references picker with a preview; hover shows the signature and docs; `cargo check` errors show a red squiggle, a gutter icon and a count in the status bar; ⇧⌘O and ⌘T find symbols; breadcrumbs follow the cursor.
- When an agent edits a file on disk: diagnostics and symbols update with no action needed.
- With no server on PATH: Lucerna is the same as before, and the ⚡ button says clearly what is missing and how to configure it.
- The Phase 1 performance goals still hold (first frame under 200 ms, idle RAM under 150 MB while no file with a server is open).

**Out of scope:** the WASM host (3b); automatic server download; the Problems tab; coloring the Explorer by errors; pull diagnostics (push only); semantic tokens, inlay hints, document highlight, code lens; everything related to editing code; multi-root workspaces.

## 2. References

Details are in `docs/research/2026-11-10-lsp.md`. A summary of the points that decided the design:
- **VS Code:** the commands and keys `editor.action.revealDefinition` (F12), `peekDefinition` (⌥F12), `goToReferences` (⇧F12, peek when there are several results), `references-view.findReferences` (⇧⌥F12), `marker.nextInFiles` (F8), `gotoSymbol` (⇧⌘O, `@`), `showAllSymbols` (⌘T, `#`), `navigateBack` (⌃-). Hover has a 300 ms delay and is sticky. If the result is at the cursor itself, Find References runs. No diagnostics icon in the gutter. **Read-only trap:** `editor.renderValidationDecorations` defaults to `editable`, so VS Code hides squiggles in a read-only editor; Lucerna always shows them.
- **Zed:** a hand-written JSON-RPC transport, requests have a 120 s timeout and automatically send `$/cancelRequest` when the future is dropped; servers start lazily per (worktree, server); shutdown 5 s then exit then kill; no automatic restart. Hover sends its request at half the delay. No peek: multiple results open a multibuffer or a picker with a preview (`lsp_locations`). Outline and breadcrumbs come from `outline.scm` by default. The status bar has an LSP button (⚡) with a status dot and a popover per server; the log is a read-only view. Restricted Mode for untrusted worktrees.
- **Lucerna differs from both** in: always showing diagnostics even though read-only (unlike VS Code); having a gutter icon (unlike both, the user's choice); Restricted Mode blocks only servers and the workspace `lsp` key, not every workspace setting (unlike Zed, see §5).

License: Zed's crates `lsp`, `project`, `editor`, `language` are GPL-3, so only the design is learned. `lsp-types` (MIT) and `pulldown-cmark` (MIT) are used as dependencies. Icons from Lucide (ISC).

## 3. Architecture

```
lucerna-lsp (new, no gpui)
  transport ── server ── registry ── adapters
                 │          │
              sync      diagnostics, symbols
lucerna-syntax: outline.scm → symbol tree
lucerna-settings: reading settings while skipping the workspace layer
lucerna-theme: diagnostics and link colors
lucerna-workbench: language_servers (bridge), diagnostics_view, hover, goto, locations_picker,
                   references_view, excerpt_list, symbols (@, #, breadcrumbs), lsp_status, trust, nav_history
```

### 3.1 `lucerna-lsp`

A new crate depending on `lsp-types`, `serde`, `serde_json`, `futures`, `crossbeam-channel`, `globset`, `tracing`, `lucerna-text` (snapshots for position conversion) and `lucerna-syntax` (the symbol tree types). No gpui, no tokio.

**`transport`**
- `Transport::spawn(command, args, env, cwd) -> Result<Transport>`: runs the child process with piped stdin/stdout/stderr, and three named threads (`lsp-read-<server>`, `lsp-write-<server>`, `lsp-stderr-<server>`), each wrapped in `catch_panic`.
- Write thread: receives JSON strings over an unbounded channel, writes `Content-Length: N\r\n\r\n` + the body, and flushes after every message.
- Read thread: reads headers up to `\r\n\r\n` (accepting `Content-Type`, in any order), `read_exact` for the body, then classifies: a response (has `id`, no `method`) is sent to the waiting oneshot; a request or notification from the server goes into an event channel bounded at 128.
- Stderr thread: reads line by line and puts them in the server's log.
- EOF on stdout: every pending request receives `Err(Closed)`, and an `Exited { code }` event is emitted.

**`server`**
- `LanguageServer`: a process that has finished `initialize`.
  - `request::<R: lsp_types::request::Request>(params) -> Request<R::Result>`. `Request` is a future: it has a timeout (default 120 s; hover and ⌘-hover 10 s), and **dropping it before a result arrives sends `$/cancelRequest`**.
  - `notify::<N>(params)`.
  - `capabilities() -> &ServerCapabilities`, `position_encoding()`.
  - `shutdown()`: `shutdown` (wait at most 5 s) → `exit` → wait 1 s for the process to leave → kill. `Drop` calls shutdown on a separate thread.
- `initialize` sends:
  - `processId`, `rootUri`, `workspaceFolders` = [root], `clientInfo { name: "Lucerna", version }`, `initializationOptions` from the adapter merged with `lsp.<server>.initialization_options`.
  - `general.positionEncodings = ["utf-8", "utf-16"]`.
  - Read-only capabilities: `definition`, `typeDefinition`, `implementation`, `declaration` (all `linkSupport`); `references`; `hover` (`contentFormat: ["markdown", "plaintext"]`); `documentSymbol` (`hierarchicalDocumentSymbolSupport`); `publishDiagnostics` (`relatedInformation`, `versionSupport`, `tagSupport [1, 2]`, `codeDescriptionSupport`); `synchronization` (no `willSave`, no `didSave`); `workspace.symbol`; `workspace.configuration`; `workspace.workspaceFolders`; `workspace.didChangeWatchedFiles` (`dynamicRegistration`, `relativePatternSupport`); `window.workDoneProgress`; `window.showMessage`.
  - After `initialized`, send `workspace/didChangeConfiguration { settings: lsp.<server>.settings }`.
- When the server sends a request across, the client answers it itself:

  | Request | Reply |
  |---|---|
  | `workspace/configuration` | For each item: `lsp.<server>.settings` (the whole object, or by `section` of the form `a.b`), `null` if absent |
  | `window/workDoneProgress/create` | `null` (OK) |
  | `client/registerCapability`, `client/unregisterCapability` | OK; record the `workspace/didChangeWatchedFiles` registration |
  | `workspace/workspaceFolders` | `[root]` |
  | `window/showMessageRequest` | `null` (no action chosen), the message goes to the log |
  | `workspace/applyEdit`, `window/showDocument` | `{ applied: false }` / `{ success: false }` |
  | anything else | `MethodNotFound` error |

- Notifications from the server: `textDocument/publishDiagnostics` → the diagnostics store; `$/progress` → status; `window/logMessage` and `window/showMessage` → the log (an Error-level showMessage also appears in the ⚡ popover); anything else is dropped.

**`adapters`**

> Replaced by the JSON table `crates/lucerna-lsp/assets/servers.json` (see `2026-11-11-lsp-server-table-design.md`). The table below holds the original values of the first six servers; those values are kept as they are in `servers.json`, except that `clangd` has `languages` set to `["c","c++","objective-c"]` (extension id = lowercase name); the root markers of `clangd` are kept (`compile_commands.json`, `.clangd`).

| Server | Languages (Lucerna id) | Binary and arguments | Root | Notes |
|---|---|---|---|---|
| `rust-analyzer` | `rust` | `rust-analyzer` | The **outermost** directory with `Cargo.toml`, counting from the file up to the workspace root; if none, the workspace root | Probe with `--version` (the rustup proxy returns an error when the component is not installed) |
| `gopls` | `go` | `gopls` | The nearest directory with `go.work`, then `go.mod`; if none, the workspace root | |
| `basedpyright` | `python` | `basedpyright-langserver --stdio` | The nearest directory with `pyproject.toml`, `setup.py`, `setup.cfg`, `pyrightconfig.json`; if none, the workspace root | Tried before pyright by default |
| `pyright` | `python` | `pyright-langserver --stdio` | as above | |
| `typescript-language-server` | `typescript`, `typescriptreact`, `javascript`, `javascriptreact` | `typescript-language-server --stdio` | The nearest directory with `tsconfig.json`, `jsconfig.json`, `package.json`; if none, the workspace root | |
| `clangd` | `c`, `cpp` | `clangd` | The nearest directory with `compile_commands.json`, `.clangd`; if none, the workspace root | Only takes effect when an extension provides the C/C++ language |

- `Adapter` is a trait: `name`, `language_ids`, `default_command`, `root_markers` + the search kind (nearest / outermost), `language_id_for(lang)` (the id sent in `didOpen`), the default `initialization_options()`, and `probe_args` (default `--version`; empty means no probe).
- **Finding the binary:** `lsp.<server>.binary.path` if present (used as is, no probe), together with `binary.arguments` (replacing the default arguments) and `binary.env`. Otherwise look up the binary name in the PATH of the **login shell environment** and run `probe_args` as a probe with a 5 s timeout. The root does not change the lookup result, so the result is cached by server name until settings change.
- **Login shell environment** (`env.rs`, separate code for each platform): macOS and Linux run `$SHELL -l -i -c 'env -0'` once, in the background, with a 5 s timeout, cached for the whole process. On an error or timeout, the process environment is used. Windows (later) uses the process environment. The server process receives this environment, with `binary.env` merged on top.

**`registry`**
- `ServerKey = (root: Arc<Path>, server: &'static str)`. Each key has a `ServerSlot`: `Starting`, `Running(Arc<LanguageServer>)`, `Failed { message, stderr_tail }`, `Stopped`, `NotFound`.
- `servers_for(language_id, path) -> Vec<ServerKey>`: takes the list from the language's `lucerna.lsp.languageServers` (§8), filters the adapters that serve that language, and computes the root per adapter.
- A server starts lazily when the first file for it is opened. While it is `Starting`, `didOpen` messages are queued and sent right after `initialized`.
- When settings change (`lsp`, `lucerna.lsp.*`), recompute: a server no longer selected is shut down; a server whose `binary`/`initialization_options` changed is restarted; if only `settings` changed, send `didChangeConfiguration`.
- `restart(key)`, `stop(key)`, `shutdown_all()` (in parallel, at most 5 s per server).

**`sync`**
- `DocumentSync` tracks, for each (server, uri): the current version and the snapshot last sent.
- `open(path, language, snapshot)`: sends `didOpen` (version 0, the whole text) to every server of the file.
- `change(path, snapshot)`: version + 1. A `Full` server receives the whole text; an `Incremental` server receives one change covering the whole old text (a range from the start to the end of the old snapshot). A `None` server receives nothing.
- `close(path)`: `didClose`, and clear the file's diagnostics if the server no longer sends them for unopened files (diagnostics of unopened files are kept; see §7.1).
- No `didOpen` when the file exceeds `lucerna.largeFile.maxBytes` / `maxLines`, or has a line longer than 20,000 characters.
- `didChangeWatchedFiles`: the `workspace/didChangeWatchedFiles` registrations are translated into `globset` (patterns relative to `baseUri`). Every worktree event (create, change, delete) that matches a glob is sent to that server, batched in 100 ms groups.

**`position`**
- `to_lsp(snapshot, Point, encoding) -> lsp::Position` and `from_lsp(snapshot, lsp::Position, encoding) -> Point`, computed on the rope's line. UTF-16 counts code units; UTF-8 counts bytes. A position out of range is clamped to the end of the line or the end of the file, with no panic.

**`diagnostics`**
- `DiagnosticStore`: `HashMap<PathBuf, HashMap<ServerKey, FileDiagnostics>>`, `FileDiagnostics { version: Option<i32>, items: Vec<Diagnostic> }`.
- `Diagnostic { range: Range<Point>, severity, message, source, code, code_href, tags: { unnecessary, deprecated } }`. The range is converted with the open snapshot; for an unopened file, keep the raw `lsp::Range` and convert it on open.
- A `publishDiagnostics` whose `version` differs from the file's most recently sent version is dropped (Lucerna is read-only, and the server will send again after `didChange`).
- `summary() -> (errors, warnings)` for the whole project; `for_file(path)` sorted by position; `next(from: (path, Point), forward, scope: File | Files)` for F8 / ⌥F8: counts only error, warning, info (hints are skipped, like VS Code); files ordered by path; wraps around to the start.
- One `DiagnosticsChanged(paths)` event per batch.

**`symbols`**
- `SymbolTree`: `Vec<Symbol>`, `Symbol { name, detail, kind: SymbolKind, range: Range<Point>, selection_range: Range<Point>, children: Vec<Symbol> }`. `SymbolKind` follows LSP.
- From LSP: `DocumentSymbol[]` (hierarchical) is used as is; `SymbolInformation[]` (flat) builds a tree by one range containing another.
- `path_at(tree, Point) -> Vec<&Symbol>`: the chain of symbols containing that point, going deeper, for breadcrumbs.
- `flatten(tree) -> Vec<(depth, ancestors, &Symbol)>`: for `@`.

### 3.2 `lucerna-syntax`: `outline.scm`

- `Language` also reads the extension's `outline.scm` (lazily, like `highlights.scm`). Bundled languages do not have one yet.
- `SyntaxState::outline(snapshot) -> Option<SymbolTree>`: runs the query on the existing tree. Each `@item` match is a symbol; `@name` is the name; `@context` (minus the part that duplicates the name) is joined into `detail`; the parent-child relation follows ranges containing each other. `kind` is guessed from the first word of `@context` (`fn`/`def`/`func`/`function` → Function, `struct`/`class` → Struct/Class, `enum`, `trait`/`interface`, `impl`, `mod`/`module`/`namespace`, `const`, `type`); if it cannot be guessed, `Object`.
- The types `SymbolTree`/`Symbol`/`SymbolKind` are defined in `lucerna-syntax` (so `lucerna-lsp` can reuse them), independent of `lsp-types`.

### 3.3 `lucerna-settings`

- `SettingsStore::get_trusted::<T>(key, language)`: like `get` but skipping the `Workspace` layer. Used for the `lsp` key when the folder is not trusted (§5).

### 3.4 `lucerna-theme`

`UiColors` gains (read from the Zed theme, with fallback colors by appearance when missing):
- `warning`, `info`, `hint` (`error` already exists);
- `error_background`, `warning_background`, `info_background`, `hint_background`;
- `error_border`, `warning_border`, `info_border`, `hint_border`;
- `link_text_hover` (`link_text.hover`);
- `editor_highlighted_line_background` (`editor.highlighted_line.background`);
- `element_hover` (`element.hover`).

The fallback colors follow One Dark / One Light (recorded in research §6.2), with the values written by hand.

### 3.5 `lucerna-workbench`

One job per module, small files:

| Module | Job |
|---|---|
| `language_servers.rs` | The `LanguageServers` entity (one per workbench): holds the `Registry`, `DocumentSync`, `DiagnosticStore`, and a `SymbolTree` cache per document; receives server events through a `cx.spawn` loop (like the worktree pump); listens to `Documents` (open, reload, close) and `on_worktree_event`; emits events to the views. Async API for views: `definition(path, point, kind)`, `references`, `hover`, `document_symbols`, `workspace_symbols(query)`. |
| `trust.rs` | The list of trusted folders, the trust modal, the "Restricted Mode" button in the title bar. |
| `nav_history.rs` | Navigation history per group, Go Back / Forward. |
| `code_view/diagnostics.rs` | Per-line diagnostics drawing data for `CodeView` (underline, gutter, inline, scrollbar). |
| `code_view/link.rs` | ⌘-hover / ⌘-click. |
| `hover.rs` | The hover and diagnostic popover. |
| `markdown.rs` | Renders markdown (pulldown-cmark) into gpui elements; code blocks are colored with `lucerna-syntax`. A shared component. |
| `goto.rs` | The commands for go to definition / type / implementation / declaration, and references; decides between jumping directly, opening a picker, or switching to references. |
| `locations_picker.rs` | The location picker with a preview. |
| `excerpt_list/` | A list of file → code excerpts split out of `search_view` (row data, file header, code line with gutter, background syntax highlighting, keyboard selection). Shared by the Search tab and the References tab. |
| `references_view.rs` | The References tab. |
| `symbols/` | The symbol source (LSP, then `outline.scm`), Quick Open `@` and `#`, breadcrumbs. |
| `lsp_status.rs` | The ⚡ button in the status bar, the server popover, progress text, the log tab. |
| `diagnostics_status.rs` | The error / warning count and the message at the cursor in the status bar. |

## 4. Server lifecycle

1. `Documents` reports that a file was opened (it already has text and a language).
2. If `lucerna.lsp.enabled` is `false`, or the folder is not trusted, stop.
3. `registry.servers_for(language, path)` → the `ServerKey`s. A key with no slot is started in the background: find the binary → spawn → `initialize` (60 s timeout) → `initialized` → `didChangeConfiguration`. Meanwhile the slot is `Starting`.
4. `didOpen` is sent immediately if the server is already `Running`, otherwise queued.
5. Binary not found: the slot is `NotFound`, the ⚡ button gets a yellow dot, and the popover says `rust-analyzer not found on PATH. Install it or set "lsp": { "rust-analyzer": { "binary": { "path": … } } }`. Reported once per server per session (not repeated every time a file is opened).
6. `initialize` fails or the process exits before that: the slot is `Failed` with the error and the last 50 lines of stderr; red dot.
7. The server exits while running: pending requests return `Err` (the UI does not report an error for each request), that server's diagnostics are cleared, and the slot is `Failed` with the exit code. No automatic restart.
8. Closing the window or quitting the app: `shutdown_all` in parallel, at most 5 s. The registry's `Drop` does this too.

## 5. Folder trust (like Zed)

- A folder opened for the first time is in **Restricted Mode**: no server is started, and the `lsp` key of `.lucerna/settings.json` is ignored (read with `get_trusted`). Other workspace settings still apply: they cannot run code. This is where Lucerna differs from Zed (Zed ignores the project's whole settings file).
- The title bar shows a text button "Restricted Mode" with a warning icon (`warning`), like Zed. Clicking it, or running the command `workbench.trust.manage`, opens the **trust modal** (using `popover`):
  - a title with a warning icon: "Unrecognized Project";
  - the folder path, in a muted color;
  - "Untrusted projects are opened in Restricted Mode to protect your system.";
  - "Restricted Mode prevents:" and two bullets "Language servers from running", "`lsp` settings in .lucerna/settings.json from being applied";
  - a checkbox "Trust all projects in the `<parent>` folder";
  - two buttons: "Stay in Restricted Mode" (Esc) and "Trust and Continue" (Enter, the primary button).
- The modal appears automatically the first time a file is opened whose language has a server on PATH, in an untrusted folder (not right when the folder is opened, so someone who only views ordinary files is not asked). After "Stay in Restricted Mode", the modal does not reappear on its own for that session.
- The choice is stored in `<data dir>/trusted.json`: `{ "folders": [..], "parents": [..] }`. A folder is trusted if it is in `folders` or lies under an entry in `parents`. The command `workbench.trust.clear` clears everything and returns windows to Restricted Mode (stopping every server).
- The setting `session.trustAllWorktrees` (default `false`, read only in the User layer): when `true`, every folder counts as trusted, and nothing is stored in `trusted.json`.
- Single-file mode (`luc file.rs`): the folder containing the file is treated as an ordinary folder.
- When the user trusts, files that are already open get `didOpen` right away.

## 6. Document sync

- **Open:** when `Documents` finishes loading a file (or, if the file already exists, when the first view opens), `LanguageServers` calls `sync.open`. A document gets `didOpen` only once even with several tabs.
- **Change on disk:** a reload in `Documents` (due to an agent writing the file, through the existing `on_worktree_event`) calls `sync.change` with the new snapshot. Not sent when a reload does not change the content.
- **Close:** when the last view of a document closes (`collect_closed`), call `sync.close`.
- **Language change** (`languages_changed`): `didClose` with the old server, `didOpen` with the new server if different.
- **File deleted:** `didClose`; the delete event still reaches `didChangeWatchedFiles`.
- `Documents` emits the events `DocumentEvent::{Opened, Reloaded, Closed, LanguageChanged}` (gpui entity events); `LanguageServers` subscribes to them.

## 7. Features and UI

Every LSP feature is on only when the file has at least one `Running` server that supports the matching capability; otherwise the command does nothing (no error shown), except symbols where `outline.scm` exists.

### 7.1 Diagnostics

**In the editor** (`CodeView`):
- **Squiggles** 1 px tall, under the range, colored by severity: `error`, `warning`, `info`, `hint`. An empty range squiggles one character (or the last character of the line). A diagnostic with the `Unnecessary` tag and a severity below warning gets no squiggle; instead its text is faded to 30% opacity. The `Deprecated` tag strikes the text through.
- **Gutter icon**: each line with an error / warning / info shows the icon of the highest severity (Lucide `circle-x`, `triangle-alert`, `info`, 12 px, colored by severity) at the left edge of the gutter, before the line number. The gutter is 16 px wider when `lucerna.diagnostics.gutterIcons` is on and the file has a server. Hints have no icon.
- **Inline** (when `lucerna.diagnostics.inline.enabled`): after the last character of the line, 4 em away, shows the first line of the message of the most severe diagnostic on the line; the background is the severity color at 5% opacity, the text is in the severity color, code font, 1 px smaller, 2 px rounded corners, 4 px horizontal padding. Clipped at the editor's right edge. Not shown for hints.
- **Scrollbar**: a horizontal mark 2 px tall in the right half of the scrollbar track, colored by severity (error, warning; info and hint are not drawn), drawn at the line's proportional position.
- The drawing data (`code_view/diagnostics.rs`) is rebuilt when the file's diagnostics change or the file reloads, not per frame: a per-line list of squiggle segments, the highest severity of the line, and the inline message.

**Diagnostic hover:** hovering the mouse over a range with a diagnostic (the narrowest range wins) adds a diagnostic block at the top of the hover (§7.2): background `<severity>.background`, 1 px border `<severity>.border`, rounded corners, 4×8 padding; the content is the message (markdown if the server sent markdown) and a muted line `source(code)`; `code` is a link if there is `codeDescription.href`. Overlapping diagnostics are listed one after another.

**Status bar** (on the left, after the panel buttons):
- `⊗ 3  ⚠ 12` (Lucide icons `circle-x` in `error` color, `triangle-alert` in `warning` color, the numbers in `text.muted`); with no errors, a muted `check` icon. Shown only when at least one server has ever run.
- Right after: the first line of the message of the diagnostic at the cursor, muted, cut at 80 characters.
- Clicking the count runs `editor.action.marker.nextInFiles`. Clicking the message runs `editor.action.marker.next`.
- Updated in 50 ms batches.

**F8:** `editor.action.marker.nextInFiles` (F8) / `prevInFiles` (⇧F8) walk through files in path order, opening the file (as a preview) when needed; `editor.action.marker.next` (⌥F8) / `prev` (⇧⌥F8) stay within the file. Place the cursor at the start of the range, scroll it to the middle, and open the diagnostic hover there immediately (no delay). Recorded in the navigation history.

### 7.2 Hover

- **Trigger:** the mouse rests on a character that has text. After `editor.hover.delay / 2`, send `textDocument/hover` (and look for a diagnostic at that spot); after the full `editor.hover.delay`, show it if a result is already in, otherwise show it as soon as the result arrives. If the mouse leaves the character before it is shown, cancel (drop the request → `$/cancelRequest`). If the mouse is still within the range of the result being shown, do not send again.
- `editor.action.showHover` (⌘K ⌘I): shows at the cursor right away, with no delay.
- **Hide:** the mouse leaves the range. When `editor.hover.sticky` is on, moving the mouse into the popover keeps it; leaving the popover or the range hides it after `editor.hover.hidingDelay`. Scrolling the editor, typing, Esc, or losing focus hides it immediately.
- **Look** (following Zed): uses the `popover` of `ui.rs` (background `elevated_surface.background`, border, rounded corners, shadow) but placed at the cursor: above the line if there is room, otherwise below; 8 px from the editor's right edge. 8 px padding. At most min(half the editor, 120 characters) wide, at least 20 characters; at most min(half the editor, 16 lines) tall, scrollable with the mouse. The diagnostic block (if any) is on top, 6 px apart. The hovered range has the background `element.hover`.
- **Content:** `MarkupContent` as markdown or plain text; the old `MarkedString` becomes a code block of the given language. Cut at 100,000 bytes. Rendered with `markdown.rs`:
  - paragraphs in the UI font; headings in bold, slightly larger; lists; muted quotes with a left bar; rules;
  - inline code: code font, `editor.background` background;
  - code blocks: code font, colored with `lucerna-syntax` by the fence's language name (the file's language if absent), no wrapping, scrolling horizontally within the block;
  - links: underlined; clicking opens the browser (`http`, `https`) or opens a file (`file://…#L10`) in Lucerna;
  - selecting and copying text: out of scope.
- Settings: `editor.hover.enabled` (`true`), `editor.hover.delay` (300), `editor.hover.sticky` (`true`), `editor.hover.hidingDelay` (300).

### 7.3 Go to definition and ⌘-click

**Commands** (`goto.rs`), all taking the cursor position of the focused editor:

| Command | Request | Multiple results |
|---|---|---|
| `editor.action.revealDefinition` | `definition` | picker |
| `editor.action.peekDefinition` | `definition` | always a picker (even with 1 result) |
| `editor.action.revealDefinitionAside` | `definition` | picker; opens in the adjacent group |
| `editor.action.goToTypeDefinition` / `peekTypeDefinition` | `typeDefinition` | picker / always a picker |
| `editor.action.goToImplementation` / `peekImplementation` | `implementation` | picker / always a picker |
| `editor.action.revealDeclaration` / `peekDeclaration` | `declaration` | picker / always a picker |

- The result is a `Location` or `LocationLink` (using `targetSelectionRange`). Merged from every server that supports it, with duplicates removed.
- **No result:** a small popover at the cursor "No definition found for '`word`'" (in the same style: "No type definition found…", "No implementation found…", "No declaration found…"), which disappears after 3 s or when you move. Not shown when run from ⌘-click.
- **A single result at the cursor position** (for a non-peek command): run `editor.action.goToReferences` instead (like VS Code's `alternativeDefinitionCommand` and Zed's `go_to_definition_fallback`).
- **A single result:** open the file with `OpenMode::Preview` (setting `workbench.editor.enablePreview`), place the cursor at the start of the range, scroll it to the middle. Record the navigation history before jumping.
- The settings `editor.gotoLocation.multipleDefinitions`, `multipleTypeDefinitions`, `multipleImplementations`, `multipleDeclarations`, `multipleReferences`: `peek` (default, opens a picker), `gotoAndPeek` (jump to the first result and open a picker), `goto` (only jump to the first result).

**⌘-hover / ⌘-click** (`code_view/link.rs`):
- Hold ⌘ (Ctrl on Linux/Windows) and move the mouse: send `definition` for that position (cancelling the old request when the position changes; cached by the result's range). If there is a result, the origin range (`originSelectionRange`, or the in-hover word if absent) gets a straight 1 px underline and changes to the `link_text.hover` color; the mouse cursor becomes a hand. Releasing ⌘ or leaving the range removes it.
- ⌘-click: go to the definition (like `revealDefinition`); ⌥⌘-click: open in the adjacent group. A ⌘-click that does not hit a range with a result keeps the old click behavior.

**Navigation history** (`nav_history.rs`):
- Each group has a `back` / `forward` stack of `(path, Point, scroll_top_row)`, at most 50 entries.
- Recorded before every jump made by a command: definition, references, symbols (`@`, `#`, breadcrumbs), F8, Quick Open (file, `:line`), Search results. A jump within the same file is recorded only when it is at least 10 lines away (Zed). A new record clears `forward`.
- `workbench.action.navigateBack` (⌃-) and `workbench.action.navigateForward` (⌃⇧-): open the file if needed (not as a preview), place the cursor, restore the scroll. If the file no longer exists, skip that entry.

### 7.4 Location picker (`locations_picker.rs`)

Following Zed's `lsp_locations`, using `popover`, `picker_input_row`, `picker_list`, `picker_row`:
- A modal positioned like Quick Open, but wider: min(90% of the window, 1100 px) wide, min(70% of the window, 560 px) tall.
- The title is in the input box (placeholder): "Filter definitions…", "Filter references…", "Filter implementations…", ….
- **Left side (40%)**: a list grouped by file. A file header row (not selectable): file icon, file name, muted directory, the number of results on the right. A result row: a muted, right-aligned line number (as wide as the largest digit count), then the line content with leading whitespace removed, code font, with the part inside the range having the `search.match_background` background. The line content is read in the background; until it is there, the row has only the line number.
- **Right side (60%)**: the preview is a read-only `CodeView` of the selected file (opened through `Documents`, sharing text and syntax with an already open tab), scrolled so the range is in the middle, the range's lines having the background `editor.highlighted_line.background`. A 1 px `border` left border.
- Typing filters fuzzily on "file name + line content". `↑`/`↓` change the result (skipping headers), and the preview follows. `Enter` opens (a preview tab), `⌘Enter` opens in the adjacent group, `Esc` closes. Clicking a row selects it; double-clicking opens it.
- Results are ordered by file (the current file first, then by path), then by position. The first result is preselected.

### 7.5 References

- `editor.action.goToReferences` (⇧F12): `textDocument/references` with `includeDeclaration: true`. One result jumps; several open a picker (per `editor.gotoLocation.multipleReferences`); none shows "No references found for '`word`'".
- `references-view.findReferences` (⇧⌥F12): opens the **References tab**:
  - at most one tab across all groups, reused each time (like the Search tab); the tab title: icon + "References to `word`"; not saved into the workspace state;
  - the top of the tab is a summary row 32 px tall following `search_toolbar`: "12 references in 4 files" and the collapse/expand-all button, and a refresh button (re-runs the request at the old position);
  - the body of the tab is the `excerpt_list`: file headers + code excerpts, each reference with 1 line of context above and below, the reference line with `search.match_background` on the range;
  - keys and mouse as in the Search tab: `↑`/`↓`, `Enter` opens, F4 / ⇧F4 (`references-view.next` / `prev`) open the next / previous result.
- **Split `excerpt_list` out of `search_view`:** the row model (`Row`, `FileResult`, `Results`, collapse/expand, next/previous step), the rendering of file headers and code lines, and the background syntax-highlighting queue move into `excerpt_list/`. The Search tab keeps the query bar, the toggles, the debounce, and streaming the results. The Search tab's behavior and look do not change; the existing Search tab tests must stay fully green. `lucerna-project::Excerpt` and `LineMatch` are shared; the References tab builds excerpts itself from the ranges (reading files in the background, merging overlapping or adjacent excerpts).

### 7.6 Symbols, Quick Open `@` and `#`, breadcrumbs

**Symbol sources of a document** (`symbols/source.rs`): if a server supports `documentSymbol`, use it; otherwise `SyntaxState::outline` (if the language has `outline.scm`); otherwise there is none. The result is cached by `(path, revision)` and recomputed after the document opens, reloads, or the server has just become ready; requests are debounced by 300 ms. Some servers return empty while indexing: that empty cache entry expires after 2 s.

**Quick Open `@`** (`workbench.action.gotoSymbol`, ⇧⌘O; type `@` in Quick Open):
- `parse_query` adds `QuickOpenQuery::Symbols { text, group_by_kind }` (`@:` groups).
- Row: a kind icon (Lucide, `icon.muted` color), the name (code font, matched characters colored `text.accent` at 30% opacity like Zed), a muted `detail` on the same line. Indented 12 px per level when not filtering. When filtering there is no indentation, but a muted ancestor name at the end of the row ("in `impl Foo`").
- Fuzzy filtering on "ancestors + name"; when the query is a single word, the matched text must be inside the name (like Zed).
- `@:`: header groups by kind "functions (12)", "structs (3)"…, ordered by `SymbolKind`.
- With no query: the deepest symbol containing the cursor is preselected.
- **Preview while moving:** selecting a row scrolls the open editor to that symbol (in the middle of the screen), and the lines from the start to the end of the symbol's `range` get the background `editor.highlighted_line.background`. `Enter` places the cursor at the start of `selectionRange` and records the navigation history. `Esc` restores the old scroll position and cursor.
- Empty states: "No symbols found" / while waiting for a server: "Loading symbols…" / no source: "No language server or outline for this file".

**Quick Open `#`** (`workbench.action.showAllSymbols`, ⌘T; type `#`):
- `QuickOpenQuery::WorkspaceSymbols(text)`. Empty query: "Type to search symbols".
- Debounce 100 ms, send `workspace/symbol { query }` to every `Running` server that has the capability; on a new query, drop the old request (it cancels itself). When results return, fuzzy-filter again on the client, at most 100, with files inside the project (not ignored) ahead of files outside.
- A two-line row (following Zed): line 1 is the kind icon + name (code font, matched text colored) + a muted `containerName`; line 2 is the relative path + `:line`, small, muted.
- `Enter` opens the file (as a preview) and places the cursor; `⌘Enter` opens in the adjacent group. No preview while moving (like VS Code).
- If a `WorkspaceSymbol` has only a `uri` (no range yet), call `workspaceSymbol/resolve` if the server supports it, otherwise open at line 1.
- No server is running: "No language server is running".

**Breadcrumbs** (`symbols/breadcrumbs.rs`):
- A row 32 px tall right below each group's tab bar, only for code tabs (not for the Search, References, Extensions tabs). Background `editor.background`, a 1 px bottom border `border_variant`, 8 px horizontal padding, small UI font size, color `text.muted`.
- Content: the file's relative path split by folder (`crates › lucerna-lsp › src › server.rs`, the file name in `text` color), then the chain of symbols at the cursor (`path_at`) with a 12 px kind icon and the name in code font. The `›` separator is in the placeholder color (`text.muted` at 60%). Beyond 12 segments, keep the first 6 and the last 6, with `⋯` in the middle. On horizontal overflow, cut on the left.
- The whole row is a button (hover `ghost_element.hover`): clicking opens Quick Open `@`. `breadcrumbs.focusAndSelect` (⇧⌘.) also opens `@`.
- Updated when the cursor changes (50 ms batches) or the symbol tree changes.
- Settings: `breadcrumbs.enabled` (`true`), `breadcrumbs.filePath` (`on` | `off` | `last`), `breadcrumbs.symbolPath` (`on` | `off` | `last`), `breadcrumbs.icons` (`true`).

### 7.7 Server status and logs (`lsp_status.rs`)

- **The ⚡ button** in the status bar on the right (before the language), using `icon_button` with the Lucide icon `zap`, shown only when at least one server is selected for the open files, or when the folder is Restricted. A 6 px dot in the corner: red (`error`) when a server is `Failed`; yellow (`warning`) when `NotFound` or Restricted; no dot when all is well. Tooltip: "Language Servers".
- **Progress text** next to the button, muted: the most recent `$/progress`, "rust-analyzer: Indexing 45%" (`title` + `percentage` + `message`, cut at 60 characters), "+2" if there are more; while starting: "Starting rust-analyzer…". Hidden when there is nothing.
- **Popover** (click the button): a list of servers by root. Each row: a status dot (green Running, yellow Starting / NotFound, red Failed, gray Stopped), the server name, a muted shortened root, and on the right the small text buttons "Restart", "Stop", "Log". A `NotFound` / `Failed` row has an extra error message line (§4). At the bottom of the popover: "Restart All", "Stop All". When Restricted: a row "Restricted Mode — Trust this folder…" opens the trust modal.
- **Log tab** (`lucerna:lsp-log/<server>`): one tab per server, read-only, code font, scrollable, auto-scrolling to the bottom when already at the bottom. Keeps the most recent 2000 lines: stderr, `window/logMessage`, `window/showMessage`, and lifecycle events (binary path, arguments, root, server version, exit code). Not saved in the workspace state. Copy everything with ⌘A ⌘C.
- Commands: `lucerna.lsp.restart` (the current file's server), `lucerna.lsp.restartAll`, `lucerna.lsp.stopAll`, `lucerna.lsp.showLog`.

### 7.8 Vim

In normal mode (when `vim.enabled`):
- `gd`: `editor.action.revealDefinition`; `gD`: `editor.action.revealDeclaration` (VSCodeVim).
- `gh`: `editor.action.showHover` (VSCodeVim).
- `]d` / `[d`: `editor.action.marker.next` / `prev` (Zed).
- `⌃o` / `⌃i` (the Vim jump list): `navigateBack` / `navigateForward`.

## 8. Commands, keys and settings

**New keys** (macOS; `default-keybindings.json`, with `editorTextFocus` in `when` for editor commands):

| Command | Key |
|---|---|
| `editor.action.revealDefinition` | `f12` |
| `editor.action.peekDefinition` | `alt+f12` |
| `editor.action.revealDefinitionAside` | `cmd+k f12` |
| `editor.action.goToImplementation` | `cmd+f12` |
| `editor.action.peekImplementation` | `shift+cmd+f12` |
| `editor.action.goToReferences` | `shift+f12` |
| `references-view.findReferences` | `shift+alt+f12` |
| `editor.action.showHover` | `cmd+k cmd+i` |
| `editor.action.marker.nextInFiles` / `prevInFiles` | `f8` / `shift+f8` |
| `editor.action.marker.next` / `prev` | `alt+f8` / `shift+alt+f8` |
| `workbench.action.gotoSymbol` | `shift+cmd+o` |
| `workbench.action.showAllSymbols` | `cmd+t` |
| `breadcrumbs.focusAndSelect` | `shift+cmd+.` |
| `workbench.action.navigateBack` / `navigateForward` | `ctrl+-` / `ctrl+shift+-` |
| `references-view.next` / `prev` | `f4` / `shift+f4` (when the References tab has focus) |

No key: `goToTypeDefinition`, `peekTypeDefinition`, `revealDeclaration`, `peekDeclaration`, `workbench.trust.manage`, `workbench.trust.clear`, `lucerna.lsp.*`. All are in the Command Palette.

**New settings** (`default-settings.json`):

| Key | Default | Notes |
|---|---|---|
| `lucerna.lsp.enabled` | `true` | Can be overridden per language (`"[rust]": { … }`) |
| `lucerna.lsp.languageServers` | `["..."]` | Per language. The order is the priority; `"!name"` disables; `"..."` is the language's remaining servers, except servers disabled by default. Default for `[python]`: `["basedpyright", "!pyright", "..."]` |
| `lsp` | `{}` | An object in Zed's shape: `"lsp": { "<server>": { "binary": { "path", "arguments", "env" }, "initialization_options", "settings" } }`, plus the server declaration fields of the language server table (`command`, `args`, `languages`, `file_types`, `root_markers`, `enabled`, `note`…; see `2026-11-11-lsp-server-table-design.md` §3.1). The workspace layer is ignored while untrusted |
| `session.trustAllWorktrees` | `false` | Read only in the User layer |
| `editor.hover.enabled` / `delay` / `sticky` / `hidingDelay` | `true` / `300` / `true` / `300` | |
| `editor.gotoLocation.multipleDefinitions` (+ `TypeDefinitions`, `Implementations`, `Declarations`, `References`) | `peek` | `peek` \| `gotoAndPeek` \| `goto` |
| `lucerna.diagnostics.gutterIcons` | `true` | |
| `lucerna.diagnostics.inline.enabled` | `false` | |
| `breadcrumbs.enabled` / `filePath` / `symbolPath` / `icons` | `true` / `on` / `on` / `true` | |

**Deviation from the spec presented during brainstorming:** the server-selection key is written `lucerna.lsp.languageServers`, overridden per language with `"[python]": { … }` (Lucerna's existing override mechanism, following VS Code), instead of Zed's `languages.<Lang>.language_servers`; the semantics of `!` and `...` are kept. `lsp.enabled` was renamed `lucerna.lsp.enabled` because `lsp` is already an object key.

**New context keys:** `referencesViewFocus` (the References tab has focus), `lspHoverVisible`.

## 9. Performance

- The UI thread does not read pipes, parse JSON, or convert positions for a whole file: the read thread parses JSON; converting diagnostics to `Point` runs on the background executor before entering the UI, already grouped by line.
- Bursts of `publishDiagnostics` (rust-analyzer sends hundreds of files at once) are merged in 50 ms batches before redrawing.
- Hover sends its request only after half the delay; ⌘-hover is cached by range. An old request is always cancelled when a new request of the same kind arrives.
- `documentSymbol` is debounced by 300 ms and cached by revision.
- Servers start only when needed; if the app has not opened any file with a server, there is no child process and no login shell is run.
- No new benchmark yet; check by hand with rust-analyzer on the Lucerna repo itself: scrolling is smooth when a file has hundreds of diagnostics.

## 10. Error handling

- Every transport thread and every `LanguageServers` callback is wrapped in `catch_panic` / `guarded`; a panic is logged and treated as the server exiting.
- Unreadable JSON: log at warn level, drop the message, do not disconnect. A bad header: treated as the server exiting.
- A failed request (`ResponseError`, timeout, server closed): the command does nothing, logged at debug level; for go to definition specifically, the popover "No definition found…" is shown only when the result is empty, not on an error.
- A position outside the file: clamped to the end of the line / end of the file.
- A setting of the wrong type (`lsp.x.binary.path` is not a string…): drop that value, use the default, and report it in the ⚡ popover like other settings errors.
- A corrupt `trusted.json`: treated as empty, rewritten when something changes.

## 11. Testing

**`lucerna-lsp`** (no real server needed):
- A `fake-lsp-server` binary (in `crates/lucerna-lsp/src/bin/`, built only for tests): reads a JSON script from an environment variable (replies by method, pushes notifications after `initialized` or after `didOpen`/`didChange`, sleeps, crashes, sends reverse requests). Writes every message received to a file for the tests to check.
- Transport: framing (`Content-Type`, several messages in one read, a large body), a response with the right id, dropping a future sends `$/cancelRequest`, timeout, EOF returns `Closed` to every pending request.
- Server: `initialize` sends the right capabilities; `workspace/configuration` is answered by `section`; `MethodNotFound` for unknown requests; shutdown in the right order, kill when the server does not answer.
- Registry: `languageServers` with `!` and `...`; roots per adapter (nested Cargo workspaces, `go.work`); changing settings restarts / sends didChangeConfiguration / stops in the right cases; `NotFound` when PATH lacks it; `binary.path` is not probed.
- Sync: the version increases; `Full` and `Incremental`; no `didOpen` for a large file; `didChangeWatchedFiles` by glob.
- Position: UTF-16 with emoji and CJK text; UTF-8; out of range.
- Diagnostics: dropped on a version mismatch; `next` across files and wrapping, skipping hints; totals.
- Symbols: a flat `SymbolInformation` becomes a tree; `path_at`.
- Tests with a real `rust-analyzer` on a small fixture crate: `#[ignore]`, run by hand (`cargo test -p lucerna-lsp -- --ignored`).

**`lucerna-syntax`:** `outline` with an `outline.scm` fixture + the existing WASM JSON grammar (or a bundled grammar + a test query), parent-child by range, guessing `kind`.

**`lucerna-settings`:** `get_trusted` skips the workspace layer, and still merges objects.

**`lucerna-workbench`** (`TestAppContext`, `fake-lsp-server`):
- Open a file → `didOpen`; an agent writes the file → `didChange`; close the last tab → `didClose`.
- Restricted Mode: no server; the modal appears when opening a file that has a server; Trust → the server runs; `trustAllWorktrees`.
- Diagnostics: underline, gutter icon, inline (on / off), the status bar count, F8 across two files, ⌥F8 within a file.
- Hover: the delay, cancelling on leave, sticky, the diagnostic block, markdown with a code block.
- F12 with one result jumps; several results open a picker; a result at the cursor runs references; none shows the popover; ⌘-click; back / forward.
- Picker: filtering, preview following the selection, Enter, ⌘Enter, Esc.
- References tab: summary, header, Enter opens the line, F4; the Search tab stays green.
- `@`: the tree, filtering, `@:`, preview and Esc restoring; the `outline.scm` fallback when there is no server.
- `#`: debounce, cancelling the old request, in-project ordering first.
- Breadcrumbs: following the cursor, clicking opens `@`, settings off.
- The ⚡ button: NotFound, Failed with stderr, Restart, the log tab.
- Vim: `gd`, `gh`, `]d`.

**UI review:** run the app with rust-analyzer on the Lucerna repo, side by side with Zed: hover, squiggles, picker, the References tab, breadcrumbs, `@`, `#`, the ⚡ button, the trust modal. Save screenshots next to the plan.

## 12. Implementation order

Each step runs on its own and has tests:
1. `lucerna-lsp`: transport, server, `fake-lsp-server`, position.
2. Adapters, login shell environment, registry, settings (`lsp`, `lucerna.lsp.*`, `get_trusted`), folder trust, the ⚡ button, the log tab.
3. `Documents` events, document sync, the diagnostics store, theme colors, drawing diagnostics (underline, gutter, inline, scrollbar), the status bar, F8.
4. `markdown.rs`, hover.
5. Go to definition (the variants), ⌘-click, the location picker, navigation history.
6. Split `excerpt_list` out of the Search tab, the References tab.
7. `outline.scm`, the symbol source, `@`, breadcrumbs, `#`.
8. Vim, UI review next to Zed, a review of the whole branch, `scripts/bundle-app.sh --install`.

## 13. Risks

| Risk | Mitigation |
|---|---|
| A new `lsp-types` release changes types (`Uri` replaces `Url` since 0.96) | Pin one version, and wrap the path ↔ URI conversion in a single function |
| The PATH of a GUI app on macOS lacks `~/.cargo/bin`, `~/go/bin`, nvm… | Take the login shell environment; `binary.path` is the escape hatch |
| rust-analyzer sends `cargo check` diagnostics for unopened files | The diagnostics store keeps unopened files too; the count and F8 include them |
| The server sends positions against an old text version while an agent writes the file continuously | Drop diagnostics on a version mismatch; the server will send again |
| Splitting out `excerpt_list` breaks the Search tab | Do it in a separate step, and the Search tab tests must stay fully green before adding References |
| A slow login shell or one that prints junk (oh-my-zsh) | A 5 s timeout, read `env -0` after a dedicated separator, and on error use the process environment |
| Tight disk | `lsp-types` and `pulldown-cmark` are small; `fake-lsp-server` is a binary of the crate, adding no dependency |

## 14. Changes to older specs

- Phase 1 spec, §1.3: add the row "3b — WASM host for Zed extensions: language servers from `extension.wasm` (PHP, Ruby, Elixir, Vue…)" between phase 3 and 4.
- Phase 2 spec, D1: "deferred to phase 3" becomes "deferred to phase 3b".
- The Extensions page: the line "Language server: Phase 3" becomes "Language server: Phase 3b".
