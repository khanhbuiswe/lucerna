# One process, many windows — design

Date: 2026-11-15. Status: approved by the user (2026-11-15).

**Points the spec author decided alone** (the user checks them at approval):
M5 (a file outside the open folders goes to the most recently used window),
M7, M10, M13 (claim the socket first), M16, M17; §6.1.3 the list of app-scope
keys and `files.associations` per window; §6.3 a standalone file window does
not accept more files; §6.5 no environment carried, no `open -a`; §6.6 the
`windows.json` format, the `recent.json` fallback, Welcome is not reopened;
§7.2 the measurement baseline is step 1.

Research: `docs/research/2026-11-15-multi-window.md` (Zed at the pinned GPUI
revision `b5cd310`, VS Code `main` branch).

## 1. Intent

Today every Lucerna window is a separate `luc` process (Welcome spec W1, W8):
workspace settings are a global tied to one root folder, so one process can
hold only one window. The consequence users see: with two windows open, the
Dock has two Lucerna icons, and App Exposé (Ctrl+↓) and ⌘` see only one
process's window. VS Code and Zed are both one app with many windows.
Lucerna is about to ship 0.1 and needs to do the same.

Success means:

- The Dock always has exactly one Lucerna icon, however many windows are
  open and from wherever they are opened (menu, Welcome, Finder, `luc` in a
  terminal).
- ⌘` cycles through every window; App Exposé shows them all.
- Each window still behaves as an independent workspace: workspace settings,
  trust, terminal, language server, layout.
- A window is no slower than the current build (§9).

## 2. Goals, non-goals, anti-goals

**Goals**

- Every window in one process. Remove `process::spawn` from every open path.
- Settings: the default and user layers are shared; the workspace layer is
  per window.
- A router that manages many windows; a single open function for every
  source.
- `luc <path>` sends the path to the running process over a Unix socket.
- Quitting and closing the last window as in VS Code.
- `window.restoreWindows: "all"`: reopen every window at startup.
- GPUI tests for multiple windows.
- Measure performance before and after.

**Non-goals (after 0.1)**

- The CLI flags `--new-window`, `--reuse-window`, `--add`, `--goto`,
  `--wait`. The socket protocol leaves room for them (§6.3).
- `window.openFoldersInNewWindow`, `window.openFilesInNewWindow`: behavior
  is fixed to VS Code's defaults (§5.3), no setting yet.
- Restoring each window's position and size. Windows still open maximized
  as today.
- Hot exit (silently saving unsaved files on quit).
- macOS native window tabs (`tabbing_identifier` stays `None`).
- Multi-root workspaces, Zed's multi-project sidebar.
- Sharing language servers between windows.
- Linux/Windows: the socket and `QuitMode` are written to be extensible, but
  only tested on macOS.

**Anti-goals** (it runs but it is still a failure)

- A window mistakenly reading another window's workspace settings, including
  in background work (auto-save, format, language server, terminal).
- One window's heavy work (repo scan, git, language server) dropping another
  window's frames.
- `luc <path>` while the app is starting up opening an extra window (Zed bug
  #61346).
- The `bench/compare` bench accidentally sending a path to the user's
  running Lucerna instead of running its own process.
- Quitting the app and losing unsaved changes without asking.

## 3. Constraints

- Behavior, settings, commands, keys follow VS Code; UI and performance
  follow Zed (`docs/design-principles.md`). Zed's crates are GPL: learn the
  design only.
- Only `lucerna-workbench` uses GPUI. No macOS assumptions outside platform
  code.
- The UI thread never waits on I/O. Reading settings files, `windows.json`
  and the socket all happen on background threads.
- GPUI at `b5cd310`: `on_app_quit` cannot cancel quitting; the test platform
  has `unimplemented!()` for `on_open_urls` and `on_reopen`; `window_stack()`
  returns `None` in tests; `active_window()` can be `None` when the app is in
  the background.
- Use the shared `target` folder; do not set `CARGO_TARGET_DIR`.

## 4. Approaches considered

### 4.1 Theme, icon theme, keymap

| Approach | Trade-off | Chosen |
|---|---|---|
| **A. App-wide** (Zed): read only from user settings | Keeps the ~140 existing color reads; every window looks the same. Diverges from VS Code: `workbench.colorTheme` in `.lucerna/settings.json` is ignored | **The user chose** |
| B. Per-window theme (VS Code) | Distinguishes projects by color; requires editing ~140 color reads and caching per theme | |

The keymap has no workspace layer in either VS Code or Lucerna, so it is
shared in both approaches.

### 4.2 Per-window workspace settings

| Approach | Trade-off | Chosen |
|---|---|---|
| **1. A separate settings entity per window** (VS Code: one `ConfigurationService` per window, passed through DI) | It is clear in the type which things belong to which window; testing two windows is easy. Handles must be passed into ~50 files | **The user chose** |
| Zed style: one global store, looked up by `SettingsLocation { worktree_id, path }` | Fits when there are settings for subfolders. Lucerna has only one file at the root, so the key is just the root folder: the key must still be passed into every entity, with an extra table lookup | |
| 2. A `WindowId` → settings table | Less passing; entities created before there is a window, or in tests, can easily read the wrong one | |
| 3. One global, swapping the workspace layer when the active window changes | Cheapest but wrong for background windows | Rejected |

### 4.3 `luc <path>` to the running app

| Approach | Trade-off | Chosen |
|---|---|---|
| **A Unix socket in the data folder** (VS Code) | Works with `cargo run` and on Linux; the bench has its own `HOME` so it has its own socket | **Chosen** |
| LaunchServices `open -a` or a `zed-cli://` URL (Zed) | Works only with the packaged build; cannot carry flags or return errors to the terminal | |

## 5. Decisions

| # | Decision | Reason |
|---|---|---|
| M1 | Every window runs in one process. `process.rs` (`SpawnRequest`, `Spawner`, `spawn`) and `open_requests::spawn_process` are deleted. | The main goal. |
| M2 | `AppState.settings` keeps only the default and user layers. The workspace layer belongs to each window's `WindowSettings` (§6.1). | The user chose approach 1. |
| M3 | Theme, icon theme, keymap, `files.associations` for building the shared `LanguageRegistry`, and the app-scope keys (§6.1.3) are read only from user settings. App-scope keys in `.lucerna/settings.json` are ignored. | The user chose A. VS Code also reads `application`-scope keys only from the user. |
| M4 | Opening a folder (or a standalone file) that already has a window focuses that window, and does not open a second copy. | VS Code `findWindowOnWorkspaceOrFolder`; avoids two windows overwriting each other's workspace state. |
| M5 | Opening a file: into the window whose root folder is the file's deepest ancestor; if none, into the **most recently used workbench window** as a tab; if there is no workbench, the open Welcome window (if any) becomes a standalone file window; otherwise open a new standalone file window. | VS Code `openFilesInNewWindow: "off"` (the default) and `findWindowOnFile`. Changed from today: a file outside the open folders no longer opens its own window when a window already exists. |
| M6 | Open Folder… and choosing an entry in Open Recent (Enter) **replace** the current window (W8 stays). ⌘Enter in Open Recent and on Welcome, and the new command `workbench.action.newWindow` (`cmd+shift+n`), open a **new window in the same process**. | VS Code `openFoldersInNewWindow: "default"`. |
| M7 | `workbench.action.newWindow` opens a new Welcome window (not Get Started). | VS Code opens an empty window, showing Welcome per `workbench.startupEditor`. |
| M8 | Closing the last window: the macOS app keeps running with the menu bar; Linux/Windows quit. Use GPUI's `QuitMode::Default` in place of the hand-written observer in `start()`. | VS Code `window-all-closed`; Zed `on_last_window_closed: platform_default`. |
| M9 | Clicking the Dock icon when no window remains (`on_reopen`): open Welcome. | VS Code opens an empty window, does not reopen the session. |
| M10 | Commands that must work with no window: Quit, New Window, Open…, Open Folder…, Open File…, Open Recent (the menu entries), Preferences (settings, keybindings), About. Register app-level `cx.on_action`; when there is a window they still run in that window as today. | GPUI sends menu actions to the focused window, and to the app-level handler if there is none. |
| M11 | ⌘Q: ask about saving window by window, the active window first; Cancel in any window cancels quitting. Write the window list (§6.6) **before** `cx.quit()`. | Like today's `workbench::quit`; Zed has the active window ask first. |
| M12 | The default of `window.restoreWindows` changes from `"one"` to `"all"`. Accepts `"preserve"`, `"all"`, `"folders"`, `"one"`, `"none"` (§6.6). | VS Code's default. |
| M13 | `luc` **claims the socket first** (`<data dir>/luc[-dev].sock`), even before starting GPUI. If it can bind, it is the app. On `EADDRINUSE` it connects as a client: sends the request, waits for the reply, then exits with code 0 (on error it prints to stderr, code 1). | VS Code `claimInstance`. Claiming first keeps two `luc` runs started nearly simultaneously from becoming two apps. Only the `-dev` suffix, so a dev build does not push paths into the release build the user is running; the name has no version, so an app of a different version with the same protocol still accepts the request, and updating Lucerna while the old app is running does not produce two apps. If the protocol is newer than the app's, `luc` prints "Lucerna was updated. Quit Lucerna and open it again." and exits 1. |
| M18 | `luc` in a terminal **detaches the app from the terminal** and returns the prompt immediately, like `code .`. The hidden flag `--foreground` keeps the old behavior (run the app right in the terminal) for `bench/compare`, `--trace-startup` and debugging. | The user chose. One process holds every window, so closing the terminal tab or pressing Ctrl+C must not take every window down with it and skip the save prompt. |
| M14 | Requests from the socket or `on_open_urls` that arrive before the app has finished reopening windows are queued, and handled after restore finishes. | Zed bug #61346. |
| M15 | Language server, terminal, git, workspace state: per window as today. Trust: a shared `trusted.json` store. Trusting a folder makes every window whose root is exactly that folder switch to trusted. Also choosing "trust the parent folder" makes every window whose root lies inside that parent switch to trusted. A window whose root is a subfolder of a folder trusted on its own stays in Restricted Mode. | Both of them do this. |
| M16 | `status_message` shows in the window that caused it. | Zed and VS Code both have per-window notifications. |
| M17 | A file open in two windows is two separate buffers. When one window saves, the other receives it through the file watcher like any outside change. | Both of them do this; the watcher and conflict handling already exist. |

## 6. Design

### 6.1 Per-window settings

#### 6.1.1 Types

- `lucerna-settings`: split the workspace layer into its own type
  `WorkspaceLayer` (the parsed map). `SettingsStore` keeps default and user;
  add `view(workspace: Option<&WorkspaceLayer>)` returning `SettingsView`, a
  type borrowing the store and the workspace layer, with `get(key, language)`
  and `get_trusted` like the store. *(The first version of the spec called it
  `get_in`; the code uses `SettingsView`.)* `SettingsStore::get(key,
  language)` keeps its meaning of "no workspace", so app-scope code and old
  tests do not change. The precedence order and object merging (R26) are
  unchanged.
- `lucerna-workbench`: `WindowSettings` is a GPUI entity owned by each
  `Workbench`. It holds:
  - the path `<root>/.lucerna/settings.json` (none for single-file);
  - the current `WorkspaceLayer`;
  - the workspace file read error;
  - a watcher on the `<root>/.lucerna` folder (not run on the test
    scheduler, like `watch_config` today);
  - a change counter.

  API: `get::<T>(key, language, cx)` merges `AppState.settings` with its own
  layer; `get_trusted` drops the workspace layer as today. It calls
  `cx.notify()` when the workspace layer changes **and** when the user layer
  changes (it observes the `AppState` global), so users only need to observe
  one thing.
- Welcome and Get Started have no `WindowSettings`: they read only
  `AppState.settings` (user).

#### 6.1.2 Passing it down

`Workbench::new` creates `WindowSettings` before any child entity and passes
the handle into every entity that needs settings: `Documents`,
`LanguageServers`, `TerminalPanel` and the terminal tab, `CodeView`, vim,
explorer, editor groups, search, markdown preview, git store, symbols, find
widget, hover, save, panel layout. Every place that does
`cx.global::<AppState>().settings.get(..)` to read a window-scope key
switches to the handle. The 8 places that `observe_global::<AppState>` to
listen for settings changes switch to `cx.observe(&settings, ..)`; places
that listen for theme still observe `AppState`.

No "read the global for speed" shortcut is kept: an architecture test (§8.3)
catches places that read window-scope keys through `AppState`.

#### 6.1.3 App scope (read only from user)

A fixed list in code (`app_scope_keys`), like VS Code's `scope: application`:

- `workbench.colorTheme`, `workbench.preferredDarkColorTheme`,
  `workbench.preferredLightColorTheme`, `window.autoDetectColorScheme`,
  `workbench.iconTheme`;
- `window.*` (including `window.restoreWindows`), `workbench.startupEditor`;
- `extensions.*`;
- `files.associations` when building the shared `LanguageRegistry` (see
  below).

App-scope keys present in `.lucerna/settings.json` are ignored, with one
`tracing::info!` logged each time the file is loaded. *(How to report this
in the UI, for example dimming the key in the editor as VS Code does, is
left for later.)*

`files.associations` in the workspace still takes effect for that window:
`WindowSettings` holds the window's `Arc<LanguageRegistry>`. If the
workspace has no `files.associations`, it uses the shared
`AppState.languages`; if it has them, the window builds its own registry
with the associations merged in. The first build, when creating
`WindowSettings`, runs immediately on the main thread so documents restored
when the window opens have the right language from the start: this only
compiles globs, reads no files, loads no grammars, so it is cheap. Later
rebuilds (the workspace file or the app's registry changing) run on a
background thread. `Documents` looks up languages through the window's
registry.

#### 6.1.4 Config files, watcher, errors

- `ConfigState` (global) keeps only the user folder: `settings.json`,
  `keybindings.json`, `themes/`. One watcher for the whole app.
- `watch_config(ConfigPaths)` loses the `workspace_settings` field; it is
  called once at startup. The router no longer calls `watch_config` again
  when the folder changes.
- Each `WindowSettings` has its own watcher for `<root>/.lucerna`, with
  debounce as today (`CONFIG_DEBOUNCE`).
- The status item reporting config errors for a window shows the user error
  (shared) and the workspace error (per window).
- `reload_config(changed)` (tests and the user watcher) is unchanged;
  `WindowSettings::reload(cx)` is for the workspace watcher. Opening
  `.lucerna/settings.json` in the editor and saving changes only that
  window.

### 6.2 The multi-window router

`Router` (in `app.rs`) replaces `shown: Option<Shown>` with a list of
windows:

```text
WindowEntry { handle: AnyWindowHandle, shown: Shown, last_active: u64 }
Shown = Welcome | Onboarding | Workbench(OpenWindow)
```

- `last_active` comes from a counter incremented each time a window is
  activated (`observe_window_activation`). "The most recently used window"
  is the one with the largest `last_active`, independent of
  `active_window()` or `window_stack()`.
- Closing a window (`on_window_closed`) removes it from the list and
  rewrites `windows.json` (§6.6).
- Changing the root in a window (W8, W10, Welcome → workbench) updates the
  `shown` of exactly that entry.
- The router is global (`Rc<RefCell<..>>` inside a `Global`), so app-level
  actions, the socket, `on_open_urls`, `on_reopen` and tests can all call
  it.

### 6.3 A single open function

```text
open(request: OpenRequest, cx) -> Option<AnyWindowHandle>
OpenRequest { paths: Vec<PathBuf>, target: Target, source: Source }
Target = Auto | NewWindow | ReplaceIn(AnyWindowHandle)
Source = Cli | Finder | Menu | Restore
```

Every source calls this function: `luc` (socket), Finder/"Open With"/Dock
(`on_open_urls`), Welcome, Open Recent, the Open dialog, New Window,
restore. The function is pure over `&mut App`, so tests can call it directly
(the test platform has no `on_open_urls`).

Rules for `Target::Auto` for each path (already canonical):

1. A folder that is already the root of a folder window, or a file that is
   already the standalone file of a standalone file window → focus that
   window (M4).
2. A file inside the root of one or more **folder windows** → the window
   with the deepest root, open the file as a pinned tab, focus the window
   (M5). A standalone file window has no root here: its parent folder is not
   scanned so it does not count.
3. A file not inside any root: if there is a folder window → the most
   recently used folder window, open as a pinned tab. If there is no folder
   window but there is Welcome/Get Started → turn the root of the most
   recently used Welcome window into a standalone file window. Otherwise →
   a new standalone file window. A standalone file window never accepts more
   files from `open`, so it is always exactly "one file" (`windows.json`
   `kind: "file"`, no workspace settings).
4. A folder not yet open: if there is Welcome/Get Started → change the root
   of the most recently used Welcome window (as in Zed: an empty window is
   reused). If not → a new window.
5. A path that does not exist → skip it and log; with `Source::Cli` return
   the error to the terminal.

`Target::NewWindow` always opens a new window, except for rule 1 (already
open means focus). `Target::ReplaceIn(w)` is W8/W10: change the root of `w`
after the old workbench has asked about saving.

After each open: record recent (as `record_recent`), `app_menu::refresh`,
write `windows.json`.

New windows open with `maximized_window_options` as today. Offsetting the
new window from the old one (cascading) is left for later.

### 6.4 Commands

| Command | Key | New behavior |
|---|---|---|
| `workbench.action.newWindow` (new) | `cmd+shift+n` | A new Welcome window (M7). In the File menu, at the start of the Open group. |
| Open Recent: Enter | | Folder: replace this window (W8, but if the folder is already open in another window, focus that window, do not replace). File: a tab here. |
| Open Recent: ⌘Enter | | `Target::NewWindow`. |
| Welcome: click an entry | | Replace this Welcome window. Already open in another window → focus that window, Welcome stays. |
| Welcome: ⌘click | | `Target::NewWindow`. |
| Open Folder… / Open… | | Replace this window (W8), except a folder already open in another window → focus. No window → a new window. |
| Close Folder (`cmd+k f`) | | As W10. |
| Get Started: "Open settings in new window" | | `Target::NewWindow` on the user's `settings.json`. |
| Quit (`cmd+q`) | | M11. |
| Closing a window (`cmd+w` when no tabs remain, the red button) | | Asks about saving that window as today; last window: M8. |

`WorkbenchEvent::Replace(SpawnRequest)` becomes
`WorkbenchEvent::Replace(Replacement)` with `Replacement = Path(PathBuf) |
Welcome`, and add `WorkbenchEvent::OpenInNewWindow(PathBuf)`.

### 6.5 `luc` from the terminal (socket)

- **Socket path:** `<data dir>/luc.sock`, `luc-dev.sock` under
  `debug_assertions`; the name has no version, so every version of the same
  build channel shares one socket. macOS `sun_path` is limited to 104 bytes:
  if longer, use `$TMPDIR/lucerna-<hash of data dir>.sock`.
- **Claiming the socket** (`crates/lucerna/src/main.rs`, right after parsing
  arguments, before `app::run`; `luc ext …` and `luc lsp …` do not touch
  the socket):
  - If `bind` succeeds: this process is the app. The listener is passed into
    `app::run`; the app accepts connections on a background thread right
    from the start, and any request arriving during startup or restore is
    queued (M14).
  - `EADDRINUSE`: try `connect` (timeout 1 s). If it connects, act as a
    client: send one JSON line, wait for one reply line (timeout 5 s), print
    the error if any, exit. `ECONNREFUSED` (a socket file left over after a
    crash) means delete the file and `bind` again once.
  - The connection is closed or errors before a reply arrives (EOF, reset):
    claim the socket again once. This is usually because another `luc` held
    the socket for a moment and then released it to the app it started. The
    second claim connects to that app, or binds itself when there is no app.
    Step 3 below also reconnects once like this.
  - An app that is quitting replies
    `{"ok":false,"error":"Lucerna is quitting","status":"quitting"}` (it
    still holds the socket for a while). `luc` re-claims the socket every
    100 ms, up to 5 s, then sends to the new app or starts the app itself;
    past the deadline it prints an error and exits with code 1 (M1-2).
  - The reply wait times out but the request has been sent (the app is busy,
    for example restoring windows): print "Lucerna is busy; the path will
    open when it is ready" and exit with code 0, because the app still opens
    the path when it is free. If opening then fails, the app reports "Could
    not open …" in the status bar of the most recently used window, since
    `luc` has already exited (the app knows because the socket connection is
    closed); if `luc` is still waiting, the error is only printed to the
    terminal (M1-3).
  - `luc` with no arguments, run in `/` while the app is running: send an
    `activate` request (field `"activate":true`, empty `paths`). The app does
    not open `/` but comes to the front as when the Dock icon is clicked
    (M9).
  - The folder holding the socket is created with mode 0700, the socket
    0600; the app accepts requests only from the same user (checking the
    peer's uid). The fallback path when too long is in the user's own folder
    (`$TMPDIR` on macOS, `$XDG_RUNTIME_DIR` or `lucerna-<uid>` elsewhere),
    not the shared `/tmp`.
  - Other errors (cannot create the data folder, permissions): print a
    warning, start the app detached from the terminal as in M18 but without
    a socket (no request sent), then exit with code 0; do not block the
    user. `--foreground`/`--trace-startup` run the app in place, with no
    socket (M1-4).
- **Detaching from the terminal** (M18). When `luc` (without `--foreground`)
  has to become the app, it does not run the app in this process but starts
  a detached app process and then sends the request to it as a client:
  1. Release the socket just claimed (so the app process can claim it).
  2. Start the app: run the same binary with the hidden flag
     `--detached-child` (replacing `--foreground`; if the child finds
     another app already holding the socket it forwards the request to it,
     M1-1, otherwise it runs the app in place), `setsid` (a new session, no
     controlling terminal), stdin/stdout/stderr to `/dev/null` (logs are
     still written to a file as today), `cwd` unchanged. Do not use
     `open -a`: it exists only for the packaged build and easily opens the
     wrong installed copy with the same bundle ID; running the binary in
     `Contents/MacOS` directly still shows exactly one bundle icon.
  3. Send the request over the socket, retrying the connect for up to 10 s
     (the app is starting up), then wait for the reply for up to 10 s more
     (the app claims the socket before the first window, so the reply waits
     through startup too); once the reply arrives, exit with code 0. If it
     cannot connect within 10 s, print an error and exit with code 1 (the
     app may still be opening). If it connects but the reply times out,
     print the "busy" message as above and exit with code 0.
  The path is resolved in the client as today, so a "path not found" error
  is still printed to the terminal before anything is started. Opened from
  Finder/Dock (`cwd` is `/`, no arguments) there is already no terminal: run
  the app in place as today.
- **`--foreground`** (hidden from `--help`): claim the socket and run the
  app in this process as the current behavior does. `bench/compare` and
  `scripts/` that launch `luc` to measure add this flag (the pidfile is still
  the app process). `--trace-startup` also runs the app in place like
  `--foreground`. Both only claim the socket, never forward the request: if
  the socket is already held they exit with code 1 and print "Lucerna is
  already running; quit it to measure startup." (`--trace-startup`) or
  "Lucerna is already running; quit it to run in the foreground."
  (`--foreground`) (M1-6).
- **App side:** for each connection: read one line on a background thread,
  send it into the router's channel on the main thread, reply after `open`
  finishes. When the app quits, delete the socket file.
- **Environment:** requests do not carry the environment variables of the
  shell that invoked `luc`. Language servers and terminals still take their
  environment from the login shell as today
  (`lucerna_lsp::env::login_shell_env`). Carrying the environment per window
  (as Zed and VS Code do) is left until after 0.1; the protocol has a
  `version` so a field can be added.
- **Protocol** (one JSON line in each direction, with `version`). The client
  keeps the connection open, without closing the write half, until the reply
  arrives; closing the connection means "stop waiting" and receives no
  reply:

  ```json
  {"version":1,"cwd":"/abs/dir","paths":["/abs/a"],"new_window":false}
  {"version":1,"cwd":"/","paths":[],"new_window":false,"activate":true}
  {"ok":true}
  {"ok":false,"error":"path not found: /abs/a"}
  {"ok":false,"error":"Lucerna is quitting","status":"quitting"}
  ```

  Empty `paths` (`luc` with no arguments) means the `cwd` folder, as today.
  `new_window` is reserved for `-n` after 0.1; 0.1 always sends `false`.
  `activate` defaults to `false` (added later, no `version` bump). `status`
  in the reply is also added later, with no `version` bump; an old `luc`
  ignores it and prints `error`. Unknown fields are ignored; a larger
  `version` gets the error "Lucerna was updated. Quit Lucerna and open it
  again."
- **Bench:** `bench/compare` sets its own `HOME` so it has its own data dir
  and socket, and still runs its own process as today. Add a check in
  `editors.py` to be sure of that (§8.4).
- A packaged build opened from Finder/Dock: macOS allows only one instance
  per bundle, so this path still goes through `on_open_urls`; the app still
  binds the socket so `luc` in a terminal can find it.

### 6.6 Quitting, closing the last window, reopening windows

**The window list** `<data dir>/windows.json`:

```json
{ "version": 1,
  "windows": [ { "kind": "folder", "path": "/abs/proj" },
               { "kind": "file", "path": "/abs/x.rs" } ] }
```

- Order: most recently used first. Only workbench windows are written;
  Welcome and Get Started are not.
- Written (on a background thread, atomically like `recent.json`) whenever a
  window is opened, closed, changes root, or changes its recency order; and
  once synchronously in Quit before `cx.quit()`. Writing continuously means
  that even after a crash the windows can still be reopened.
- Closing a window (not quitting) removes it from the list, as Zed and VS
  Code do. Closing the last window and then quitting leaves the list empty.

**Quit** (M11): `workbench::quit` keeps its algorithm, adding: sort windows
by `last_active` descending; after every window agrees, write `windows.json`
synchronously, then `cx.quit()`. A system-initiated quit (logout) runs only
`on_app_quit`: then it cannot ask, and only saves the layout as today; the
window list is already correct because it is written continuously.

**Closing the last window** (M8, M9): `QuitMode::Default`. Remove the
`on_window_closed → quit` observer in `start()`. Register `on_reopen` to
open Welcome. The socket, the user watcher and the extension service keep
running.

**Startup with no paths** (`first_screen` extended):

| `window.restoreWindows` | Opens |
|---|---|
| `"all"` (default), `"preserve"` | every window in `windows.json` that still exists |
| `"folders"` | only folder windows |
| `"one"` | the first window in the list |
| `"none"` | reopens nothing |

- An empty list or every entry gone: for every value except `"none"`, open
  the newest entry in `recent.json` that still exists (the current
  behavior). If there is nothing, Get Started (first time), Welcome, or the
  folder chooser box as today.
- Open in reverse order (oldest first) so the most recently used window ends
  up on top, as in Zed.
- No `windows.json` (first run of the new build): the same as an empty list.

**Startup with paths** (`luc <path>` when there is no app yet):
`"preserve"` reopens every window and then opens the extra path; the other
values open only the path, as VS Code does.

**Queue** (M14): the router has a `restoring` flag. During it, requests
from the socket and `on_open_urls` are queued; when restore finishes they
are handled one by one through `open` (rule 1 avoids opening duplicates).
The initial wait `OPEN_REQUEST_GRACE` for "Open With" is unchanged.

### 6.7 Trust, terminal, language server, workspace state

- **Trust:** `TrustStore` is still a shared file, and each window evaluates
  against its own root (already the case). Add an app-level "trust changed"
  event: whichever window writes `trusted.json` emits it; every workbench
  re-reads the trust of its root (on a background thread) and starts the
  language server if it is now trusted.
- **Language server:** each `Workbench` has its own `LanguageServers` as
  today; two windows with the same language run two servers.
  `QuitShutdown` already covers every window. Read `lsp` and
  `lucerna.lsp.enabled` through the window's `WindowSettings`
  (`get_trusted` for the code-running parts).
- **Terminal:** `TerminalPanel` is per window as today; the
  `terminal.integrated.*` settings are read through `WindowSettings`.
  `DefaultTerminalSpawner` and `SystemShellGlobal` hold no window state and
  stay global.
- **Workspace state:** keyed by root as today. M4 guarantees at most one
  window per root, so they do not overwrite each other.
- **`status_message`:** `StatusMessage` holds a map `WindowId → (text,
  generation)`. `show(window_id, text, cx)`; callers in the workbench pass
  their own window; the router passes the target window.
- **Recent, `RecentStore`, `ExtensionService`, theme, icon, keymap:** global
  as today.
- **Menu bar:** `app_menu::refresh` is already based on the active window;
  when there is no window, items that need a workbench are disabled and the
  M10 items stay enabled.

### 6.8 Crashes

GPUI does not isolate panics per window: a panic on the main thread inside
an AppKit callback cannot unwind across FFI and brings down the whole app.
This is the main trade-off of the single-process model. Risk reduction:

- `windows.json` and each window's layout are written continuously, so the
  next startup reopens all the windows.
- Heavy work (file scanning, git, language server, search) runs on
  background threads; a panic there is caught (`tasks::spawn_guarded`
  already exists) and only logged.
- If `luc` cannot connect to the socket it runs the app itself, so a crashed
  old app does not block the user.
- The existing panic hook writes a log; unchanged.

## 7. Performance

### 7.1 Measuring

On a release build, using `bench/compare` (`footprint` for RAM), before the
change (the branch's base commit) and after:

1. Total RAM (footprint of the process and its children) with 1, 2, 3
   windows open on the same set of repos. For the old build: sum the
   processes.
2. Time to open the second window: from sending the request (the second
   `luc <path>`) to the new window's first frame.
3. Frames dropped in window A (scrolling) while window B scans a large repo
   (opens a new folder) and while B's language server is indexing.

The measurement script lives in `bench/compare` (add a multi-window
scenario); results are recorded in `docs/benchmarks.md`.

### 7.2 Acceptance thresholds

The baseline is the measurement from step 1 (§9) on the branch's base
commit, same machine, same set of repos. The startup numbers published
earlier were retracted (commit `58540ca`), so they are not used as the
baseline.

- One window: startup time and RAM are no worse than the baseline (within
  `bench/compare`'s measurement error).
- The second window opens faster than the old build (no process startup).
- RAM with 2–3 windows: not more than 10 % above the old build's total, and
  with 3 windows it must be below the old build's total. *(Amended
  2026-11-16 by the user's decision; the first version said "lower than the
  old build's total". Each window's `IOSurface` paint buffer takes most of
  the RAM and fluctuates by about 20 MB when the app is idle, so five
  measurement rounds could not separate the savings of a few MB from merging
  the processes.)*
- Window A drops no more frames than when B is idle, within error.
- All of a window's heavy work runs on background threads.

If the thresholds are not met it is not considered done.

## 8. Tests

### 8.1 Unit

- `lucerna-settings`: `get_in` with and without a workspace layer; the
  `[lang]` key; object merging; `get_trusted` dropping the workspace.
- The window-selection rules of `open` (a pure function over a fake window
  list): focus on an identical root, the deepest root, the most recently
  used window, reusing Welcome, a new window.
- `windows.json`: read, write, corrupt file, unknown version; filtering for
  each `window.restoreWindows` value; the `recent.json` fallback.
- The socket protocol: parsing, version, a path longer than 104 bytes.

### 8.2 GPUI multi-window (`tests/multi_window.rs`)

Use `TestAppContext`, `cx.windows()`, `simulate_close`,
`simulate_prompt_answer`:

1. Open two folders → two workbench windows in one app; reopen the first
   folder → still two windows, window one is activated.
2. Two folders with different `.lucerna/settings.json` (`editor.tabSize`) →
   each window reads its own value; each window's `Documents` and
   `CodeView` use the right value. Change window A's file
   (`WindowSettings::reload`) → B does not change.
3. Change user settings (`reload_config`) → both windows see it.
4. An app-scope key in the workspace (`workbench.colorTheme`) is ignored.
5. Open a file inside window B's folder while A is active → the tab opens in
   B.
6. New Window → one more Welcome window; choose a folder on that Welcome →
   that window changes root, the window count does not change.
7. Quit with two windows that have unsaved files: answer Cancel in the
   second window → does not quit, both remain; Don't Save in both → quits
   and `windows.json` has two windows in most-recently-used order.
8. Close one of the two windows → `windows.json` has one; close the last
   window → the app does not quit (`QuitMode` set to Explicit in the test).
9. Restore: `windows.json` has three entries, one deleted → opens two
   windows, in the right order; `"folders"` drops the file entry; `"one"`
   opens one.
10. An open request arriving during restore → queued, no duplicate opened.
11. Trust in window A (root `/t`): choose "trust the parent folder" → window
    B with root `/t/sub` switches to trusted. Trust `/t` alone → B stays in
    Restricted Mode, while another window whose root is exactly `/t`
    switches to trusted.
12. A's `status_message` does not show in B.
13. The settings split does not leak: every old workbench is freed when its
    window closes (like the W8 test).

### 8.3 Architecture

A test that scans the source of `lucerna-workbench/src` (like the other
architecture tests if there are any; otherwise add it): there is no
`process::spawn` left, and no window-scope key is read through
`cx.global::<AppState>().settings` outside the allowed files
(`config_files.rs`, `app.rs`, `startup.rs`, `extensions/service.rs`,
`theme_picker.rs`).

### 8.4 `luc` integration

- Unit (no display needed): the socket-claim decision (can bind → app;
  `EADDRINUSE` + can connect → client; `ECONNREFUSED` → delete then bind
  again) on a temporary socket; the client sends/receives one line against a
  fake server.
- Integration (a real process, temporary `HOME`, `#[ignore]` because it needs
  a display; run by hand on macOS before finishing): `luc <dir>` returns the
  prompt in < 10 s and leaves exactly one app process; a second `luc <dir2>`
  exits with code 0, adding no process; `luc <nonexistent>` exits with code 1
  and prints an error; `luc --foreground <dir>` runs the app in the calling
  process.

## 9. Steps (skeleton for the plan)

Each step builds and tests on its own:

1. **Measure first** on `main`: a multi-window scenario in `bench/compare`,
   record the numbers.
2. **`WorkspaceLayer` and `get_in`** in `lucerna-settings`.
3. **`WindowSettings`** and splitting `ConfigState`: the workspace layer out
   of the global; the workbench creates the entity; still one window per
   process. Pass the handle module by module (can be split into several
   tasks: documents/code view, language servers, terminal, explorer/editor
   groups/search, the rest).
4. **The multi-window router** and `open()`; remove `process.rs`; New
   Window; per-window `status_message`.
5. **Quit, closing the last window, `on_reopen`, app-level actions.**
6. **`windows.json` and `window.restoreWindows: "all"`**, the queue during
   restore.
7. **The socket for `luc`, detaching from the terminal, `--foreground`;**
   update `bench/compare` and the shim in `scripts/bundle-app.sh` (comment).
8. **Trust synchronized between windows.**
9. **Measure after**, compare against the thresholds, update
   `docs/benchmarks.md`; update the Welcome spec (W1, W8 note "replaced by
   this spec"), the restore-last-workspace spec, and the README if it
   mentions multiple processes.

## 10. Risks and known limits

- One panic brings down every window (§6.8).
- Editing ~90 settings reads and ~40 test files: the biggest risk is
  missing a place that wrongly reads the global. The architecture test
  (§8.3) and test 2 (§8.2) are there to catch it.
- Replacing `QuitMode` changes behavior: macOS no longer quits when the last
  window closes. Any bench that relies on closing the window to end the
  process must kill instead of closing the window (check `bench/compare`).
- The old build (many processes) and the new build running together during
  an upgrade: app 0.1.0 uses `luc-0.1.0.sock`, so the first update to the
  versionless build can still have two processes (once); after that, no
  more.
- `"preserve"` differs from `"all"` only for startup with paths.
- A narrow case: the app receives a request just before the quit hooks run,
  and then the main thread is blocked for more than 5 seconds shutting down
  language servers. `luc` prints "will open when ready" and exits with code
  0, but the app quits without opening the path. An accepted limit.

## 11. Left to the plan writer

- Names of new modules (`window_settings.rs`, `windows_file.rs`,
  `cli_socket.rs` …) and how to split the tasks in step 3.
- How `Documents` gets the window's language registry (§6.1.3).
- UI details of the "New Window" item in the File menu and the Dock menu
  (`set_dock_menu`, possibly adding "New Window" as Zed does).
