# Research: one process, many windows

Date 2026-11-15. Research for moving Lucerna from "one `luc` process per window" to "one process, many windows".

Sources:
- Zed: `~/.cargo/git/checkouts/zed-a70e2ad075855582/b5cd310/` (rev `b5cd310b3b`, the same rev Lucerna pins for gpui). Zed crates other than gpui are GPL: here only the design, identifier names and a few short lines are recorded.
- GPUI (Apache-2.0): `crates/gpui`, `crates/gpui_macos` at the same rev.
- VS Code: `microsoft/vscode` branch `main`, fetched on 2026-11-15 (source files under `src/vs/...`, paths given below).

Paths starting with `crates/` below are relative to the Zed checkout.

Important note about this Zed rev: Zed now has **`MultiWorkspace`** as the root view of every window. One window can hold several `Workspace`s (projects), switched through a sidebar ("threads sidebar"). So "open folder" in Zed can mean (a) a new window, (b) adding a new workspace to the sidebar of the current window, or (c) activating an existing workspace. Older Zed (and VS Code) only have (a) and "replace window". Lucerna does not need a sidebar; where the sidebar is irrelevant, read `MultiWorkspace` below as "window".

---

## 1. Zed: how the app holds many windows

### 1.1 Shared state and per-window state

- `AppState` (`crates/workspace/src/workspace.rs:1329`) is an `Arc` stored as a GPUI global (`GlobalAppState`). Shared by every window: `languages: Arc<LanguageRegistry>`, `client`, `user_store`, `workspace_store`, `fs`, `build_window_options: fn(Option<Uuid>, &mut App) -> WindowOptions`, `node_runtime`, `session: Entity<AppSession>`.
- `WorkspaceStore` (`workspace.rs:1361`) is just a set of `(AnyWindowHandle, WeakEntity<Workspace>)` used for collaboration/follow; it is **not** the source of truth for "which windows exist". Everywhere that needs to list windows uses `cx.windows()` and then downcasts to `WindowHandle<MultiWorkspace>` (for example `workspace_windows_for_location`, `workspace.rs:11094`).
- Per window: `MultiWorkspace` (`crates/workspace/src/multi_workspace.rs:305`) → one or more `Workspace`s → each `Workspace` owns a `Project` entity.
- Per project (in practice per window): `Project` owns `BufferStore`, `LspStore`, `WorktreeStore`, `terminals`, the git store, the task store (`crates/project/src/project.rs:247-255`). Two windows on overlapping folders have two independent `Project`s, hence two buffer stores and two sets of language servers.

### 1.2 `workspace::open_paths` and how a window is chosen

`open_paths(abs_paths, app_state, OpenOptions, cx)` (`workspace.rs:11436`) is the single entry point for the CLI, opening from Finder/Dock, the Open dialog and session restore. The steps:

1. `find_existing_workspace` (`workspace.rs:11158`) walks `cx.windows()` → every workspace in every window, asks the project through `visibility_for_paths` / `visibility_for_subpaths`, and keeps the best match. How wide the match is comes from `OpenOptions.workspace_matching` (`WorkspaceMatching`, `workspace.rs:11254`): `None` (always open new), `MatchExact` (default: the path is a worktree root, or is a file/folder inside a worktree), `MatchSubpaths` (inside, but not the root itself), `MatchSubdirectory` (`zed -a`: inside any worktree, otherwise fall back to any window).
2. If nothing matches and **every path is a file**, `preferred_target_for_files` (`workspace.rs:11118`) picks the workspace of the active window, otherwise any other window. So a lone file never creates a new window when a window already exists.
3. If nothing matches and the paths include folders, the `add_dirs_to_sidebar` flag (taken from a setting) decides whether the folder becomes a new workspace in the active window or opens a new window.
4. Otherwise `Workspace::new_local(..., requesting_window, ..., open_mode)` (`workspace.rs:2160`) creates a new `Project` and puts it in `requesting_window` (`OpenMode::Activate`/`Add`) or opens a new OS window (`OpenMode::NewWindow`, `workspace.rs:1690`).

`OpenOptions` (`workspace.rs:11272`): `visible`, `focus`, `workspace_matching`, `add_dirs_to_sidebar`, `wait`, `requesting_window`, `open_mode`, `env`, `open_in_dev_container`. `should_reuse_existing_window()` returns false for `WorkspaceMatching::None | MatchSubpaths` or `OpenMode::NewWindow`.

### 1.3 "Open folder" from inside the app

The `Open` action (`workspace.rs:~997`, `prompt_and_open_paths`): the `create_new_window` flag comes from the action or from the `default_open_behavior` setting (`existing_window` default | `new_window`).
- Not a new window → `multi_workspace.open_project(paths, OpenMode::Activate)` (add to the current window).
- New window → `Workspace::open_workspace_for_paths(OpenMode::NewWindow, …)` (`workspace.rs:4072`). This function has a rule worth learning: **if the current workspace is empty (no worktree, no remote, no dirty item), it is downgraded to `Activate`, meaning this empty window is reused instead of being left behind.**
- "Replace window" (old Zed behavior, still callable through `zed -r` / `OpenBehavior::Reuse`) closes the current project with `CloseIntent::ReplaceWindow` and loads the new project into the same window. Tests: `test_replace_window_without_worktrees_prompts`, `test_replace_window_with_worktrees_hot_exits` (`workspace.rs:13857/13895`).

### 1.4 New window (cmd-shift-n)

- Keymap: `cmd-shift-n` → `workspace::NewWindow` (`assets/keymaps/default-macos.json:722`), `ctrl-shift-n` on Linux.
- Handler (`crates/zed/src/zed.rs:1337`): `open_new(Default::default(), …)` → `Workspace::new_local(vec![], …)` with an empty `Project`, then `initialize_new_window` (`zed.rs:2418`) according to `on_new_window`: `launchpad` (default: the recent-projects page in an empty pane), `empty_tab` (one untitled buffer), `readme`.
- The Dock menu's "New Window" entry is set with `cx.set_dock_menu` (`zed.rs:2340`).
- Size/position for a new window (`workspace.rs:~2320`): override from env → the workspace's own saved bounds and display (reopened project) → `read_default_window_bounds` (the most recent bounds of any window) → GPUI's default (cascading).

## 2. Zed: the `zed` CLI and the running app

Files: `crates/cli/src/cli.rs`, `crates/cli/src/main.rs`, `crates/zed/src/zed/open_listener.rs`, `crates/zed/src/zed/mac_only_instance.rs`, `crates/zed/src/main.rs`.

- The CLI is a **separate small binary**. It creates an `ipc_channel::IpcOneShotServer<IpcHandshake>` and builds the URL `zed-cli://<server_name>` (`cli/src/main.rs:537`).
- macOS: it calls `LSOpenFromURLSpec` with the app bundle as `appURL` and the `zed-cli://…` URL as the item, flags `kLSLaunchDefaults | kLSLaunchDontSwitch` (`cli/src/main.rs:1334`). LaunchServices either starts Zed.app or hands the URL to the running instance; both paths end up at GPUI's `application:openURLs:` → `App::on_open_urls` → `OpenListener` (`zed/src/main.rs:457`). (The bundle only declares `zed://`, `crates/zed/Cargo.toml:293`; `zed-cli://` works because the target app is called by name.)
- Linux: the app binds a `UnixDatagram` at `<data_dir>/zed-<channel>.sock` (`open_listener.rs:409`), deleting the stale socket on `ConnectionRefused`; the CLI sends the `zed-cli://` URL into it, or spawns the binary if nobody is listening.
- The app parses `zed-cli://name` (`open_listener.rs:158`), connects back (`connect_to_cli`, `open_listener.rs:433`) and exchanges typed messages: `CliRequest::Open { paths, urls, diff_paths, wait, open_behavior, env, cwd, … }` and `CliResponse::{Ping, Stdout, Stderr, Exit{status}}` (`cli/src/cli.rs:55-80`). This is how the CLI gets real stdout/stderr and the exit status from the app.
- Flags → `cli::OpenBehavior` (`cli/src/cli.rs:18`): `-n/--new` `AlwaysNew`, `-a/--add` `Add`, `-e/--existing` `ExistingWindow`, `--classic` (hidden; folders open a new window, files reuse a window: the behavior from before the sidebar) and `-r/--reuse` (hidden; replaces a window's workspace). No flag → `Default`, decided from `cli_default_open_behavior` (`existing_window` | `new_window`); the first time Zed is used it may even ask and write the setting (`resolve_open_behavior`, `open_listener.rs:688`). The mapping to `OpenOptions` is in `open_options_for_behavior` (`open_listener.rs:790`).
- `--wait` (`open_listener.rs:~1005`): for files, it collects the `item.on_release` futures; for a folder or no paths, it waits for the workspace entity to be released (window closed). The CLI holds the connection until then and then receives `Exit`.
- No paths and no `-n` → `restore_or_create_workspace` (same as Dock reopen).
- **Single instance:** macOS uses a TCP listener on 127.0.0.1 with a port derived from release channel + uid (`mac_only_instance.rs`): if the handshake string comes back, another instance exists, and the new process prints "zed is already running" and exits. This stops users from running the binary twice; the normal CLI goes through LaunchServices. The dev channel and `ZED_STATELESS` skip the check (`main.rs:354`).
- Cold-start race: on a cold launch, a CLI request can arrive while the session is still being restored; Zed waits for restore to finish (or for the first window) before matching paths, otherwise it would open a duplicate window (a comment cites issue #61346, `main.rs:~985`).

## 3. Zed: per-worktree and global settings

- One global `SettingsStore` (`crates/settings/src/settings_store.rs:154`) with layers: default, user, global, extension, server, and `local_settings: BTreeMap<(WorktreeId, Arc<RelPath>), SettingsContent>`. `WorktreeId` is unique across the whole process, so one store serves every window.
- `.zed/settings.json` (`paths::local_settings_file_relative_path`) can sit in any directory inside the worktree; `Project` watches them and calls `set_local_settings(worktree_id, dir, kind, content)` (`project_settings.rs:1658`). Kinds: `Settings`, `Tasks`, `Debug`, `Editorconfig`.
- Local files are parsed only as `ProjectSettingsContent` (`settings_store.rs:~1110`), a subset of `SettingsContent` (`settings_content.rs:174` flattens `project` + `theme` + …). So theme, keymap and whole-UI options **cannot** be set per project; they are global.
- Lookup: `Settings::get(Some(SettingsLocation { worktree_id, path }), cx)`; `value_for_path` (`settings_store.rs:1653`) walks the local values backwards and returns the deepest directory whose path is a prefix, otherwise the merged global value. `get_global(cx)` = `get(None)`. Editor/language settings pass the buffer's file location; app-wide code (window close behavior, restore) calls `get_global`.
- The keymap is global (`cx.bind_keys` after `cx.clear_key_bindings`), the theme is global too; menus are rebuilt after the keymap reloads (`reload_menus`).

## 4. Zed: restore on startup

- Setting `restore_on_startup` (`settings_content/src/workspace.rs:525`): `last_session` (default), `last_workspace`, `empty_tab`, `launchpad` (`assets/settings/default.json:171` still says `none`).
- Session tracking (`crates/session/src/session.rs`): each run has a UUID `session_id`; the previous one is kept as `last_session_id` in the KV store. `AppSession` polls `cx.window_stack()` and saves the window order (`session_window_stack`), then saves once more in `app_will_quit`.
- The workspace DB (`crates/workspace/src/persistence.rs`) stores each workspace with `session_id`, `window_id`, `window_bounds`, `display` (uuid), docks, and open items. `prepare_windows_to_quit` flushes serialization before quitting (`workspace.rs:12112`). When a window is closed (not a quit), the workspace is removed from the session (`remove_from_session`), so only windows still open at quit time are restored.
- `restore_or_create_workspace` (`crates/zed/src/main.rs:1422`): `restorable_workspace_locations` → for `last_session`, `last_session_workspace_locations(db, last_session_id, window_stack)`, then **reversed** so the frontmost window is opened last and ends up on top (`main.rs:~1640`); workspaces are grouped into windows by `window_id` (`persistence.rs:332`). Falls back to `last_workspace` when there is no previous session. Errors show a toast; if no window survives, an empty window is opened.
- Each restored window has its own saved bounds and display (see 1.4). Restored remote windows use `WorkspaceMatching::None` so two windows sharing a root are both restored.
- The same function runs for Dock reopen and `zed` with no paths.

## 5. Zed: quitting and closing the last window

- `Quit` (`crates/zed/src/zed.rs:1754`): an optional `confirm_quit` prompt; then windows are sorted so the **active window asks first**, then `prepare_windows_to_quit` (`workspace.rs:12075`) runs `prepare_window_to_close(window, CloseIntent::Quit)` **one window at a time**. Any cancel stops the quit (the windows already prepared are flushed; the quit is canceled).
- `Workspace::prepare_to_close` (`workspace.rs:3733`): with `CloseIntent::Quit` hot exit is allowed: dirty buffers are serialized to the DB silently and restored next run; no save prompt. For an ordinary window close, hot exit is allowed only if the workspace can be found again (it has a visible worktree, or it is the last window and closing it quits); otherwise a save/discard prompt appears.
- The window close button: `window.on_window_should_close` returns `false` and runs `MultiWorkspace::close_window` asynchronously (prompt, flush, then `window.remove_window()`), `zed.rs:494`, `multi_workspace.rs:555`.
- Closing the last window: setting `on_last_window_closed` = `platform_default` (macOS: keep running; other platforms: quit) | `quit_app` (`settings_content/src/workspace.rs:760`). Installed in `bind_on_window_closed` (`zed.rs:336`): `cx.on_window_closed(|cx, _| if cx.windows().is_empty() { cx.quit() })`, rebound when settings change. `closing_last_window_quits_app` (`workspace_settings.rs:53`) is the shared predicate (always true outside macOS).
- `when_closing_with_no_tabs` (`platform_default` = close the window on macOS, keep it elsewhere): closing the last tab closes the window.
- Clicking the Dock with no windows left: `app.on_reopen` → `restore_or_create_workspace` (`main.rs:467`).

## 6. Zed: terminal, language server, trust

- Terminal: belongs to `Project.terminals`; the terminal panel is per `Workspace`. Not shared between windows.
- Language server: `LspStore` is per `Project` (`LocalLspStore.language_servers`, `lsp_store.rs:~359`). `LanguageRegistry` (adapters, grammars, downloads) is shared through `AppState.languages`, but **server processes are not shared between windows**: two Rust windows run two rust-analyzers.
- Trust (`crates/project/src/trusted_worktrees.rs`): a global `TrustedWorktrees` entity, initialized once from the DB (`fetch_trusted_worktrees`, `main.rs:~487`). Trust is by worktree path (single file < directory < override at a parent path), persisted and shared by every window; each `Workspace` subscribes to `TrustedWorktreesEvent::Trusted` and re-serializes. A restricted project shows a security modal on open (`show_worktree_trust_security_modal`, `workspace.rs:~2440`).

## 7. GPUI at b5cd310 (Apache-2.0): the multi-window API

Everything is in `crates/gpui/src/app.rs` unless noted otherwise.

| API | Notes |
|---|---|
| `cx.open_window(options, build_root)` (`app.rs:1605`) | returns `WindowHandle<V>`; draws once before returning. |
| `cx.windows()` (`:1581`) | every open window as `AnyWindowHandle`; `downcast::<V>()` to filter by root view. The order is slot-map order, not z-order. |
| `cx.window_stack()` (`:1593`) | front-to-back order; an `Option` because a platform may not implement it (the trait's default impl returns `None`, `platform.rs:402`; macOS uses `NSApp orderedWindows`). |
| `cx.active_window()` (`:1598`) | the platform's key window. Can be `None` when the app is not in the foreground, so fall back to `window_stack().first()` or the most recently focused window that you track yourself. |
| `cx.on_window_closed(\|cx, WindowId\|)` (`:2867`) | runs after the window has been removed (no longer accessible). |
| `QuitMode` + `Application::with_quit_mode` / `cx.set_quit_mode` (`:526`, `:291`, `:2096`) | `Default` = `Explicit` on macOS, `LastWindowClosed` elsewhere; checked right after the closed observers (`:3294`). Lucerna could use this instead of a hand-written `on_window_closed` quit. |
| `Application::on_reopen` (`:369`) | macOS `applicationShouldHandleReopen:hasVisibleWindows:`; GPUI only calls back **when no window is visible** (`gpui_macos/src/platform.rs:1373`). |
| `Application::on_open_urls` (`:359`) | macOS `application:openURLs:` (Finder Open With, `open -a`, LaunchServices URLs including `file://`). Must be registered before `run`. |
| `cx.on_app_quit(\|cx\| future)` (`:2842`) | runs in `shutdown`, the future is given `SHUTDOWN_TIMEOUT`; **cannot be canceled**. There is no "should quit" hook to veto: Zed vetoes by doing every prompt in the `Quit` action *before* calling `cx.quit()`. `Platform::on_quit` hooks into `applicationWillTerminate:`. |
| `window.on_window_should_close(cx, \|window, cx\| -> bool)` (`window.rs:7162`) | per-window veto for the close button; return `false` and close later with `window.remove_window()` (`window.rs:2599`). |
| `cx.quit()` (`:1343`) | macOS: asynchronous `[NSApp terminate:]`. Note: a system-initiated quit (logout, Dock "Quit") bypasses your `Quit` action; only `on_app_quit` runs then. |
| `cx.set_menus(..)` (`:2902`) | one menu bar for the whole app; a menu item sends its action to the focused window, otherwise to the global `cx.on_action` handler. Actions that must work with no window (Quit, Open, New Window, Open Recent) need a global handler. |
| `cx.set_dock_menu(..)`, `cx.add_recent_document(path)` | Dock menu / recent documents. |
| `cx.activate(ignoring_other_apps)`, `window.activate_window()` | bring the app / one window to the front. |
| `WindowOptions.tabbing_identifier` (`gpui_macos/src/window.rs:1011, 1248, 1290`) | `Some(id)` enables `NSWindow` automatic tabs; a new window joins the main window as a native tab when the user's system "prefer tabs" option is Always (or InFullScreen while in fullscreen). `None` turns tabs off globally. Zed exposes it as `use_system_window_tabs`. |

Test support (`crates/gpui/src/app/test_context.rs`, `crates/gpui/src/platform/test/platform.rs`):
- `TestAppContext::add_window(|window, cx| V)` → `WindowHandle<V>`; `add_window_view` / `add_empty_window` → `&mut VisualTestContext`; `cx.windows()`; `cx.update_window(..)`; `run_until_parked()`.
- `VisualTestContext::simulate_close()` calls the window's should-close handler and returns whether it allowed the close.
- `simulate_prompt_answer(button)`, `has_pending_prompt()`, `simulate_path_prompt_response` for prompt flows (for example a dirty-buffer prompt across windows).
- `TestPlatform::active_window` is a `RefCell` set when a test window is activated (`window.rs:423`), so `cx.active_window()` works in tests; `window_stack()` returns `None` (default impl).
- `TestPlatform::on_open_urls` and `on_reopen` are **`unimplemented!()`** and `quit()` does nothing. Keep the routing logic in ordinary functions that take `&mut App` (like Zed's `handle_open_request`, `restore_or_create_workspace`) and test those; only the thin `Application::on_*` glue goes untested.
- `TestAppContext::new_app()` builds a second app on the same dispatcher (useful for "second instance"-style tests).

## 8. VS Code

Files (microsoft/vscode `main`): `src/vs/platform/windows/electron-main/windowsMainService.ts`, `windowsFinder.ts`, `windowsStateHandler.ts`, `src/vs/platform/launch/electron-main/launchMainService.ts`, `src/vs/code/electron-main/main.ts`, `app.ts`, `src/vs/platform/lifecycle/electron-main/lifecycleMainService.ts`, `src/vs/code/node/cli.ts`, `src/vs/platform/environment/node/argv.ts`, `src/vs/workbench/browser/workbench.contribution.ts`, `src/vs/workbench/electron-browser/desktop.contribution.ts`, `src/vs/workbench/contrib/files/browser/files.contribution.ts`, `src/vs/workbench/services/workingCopy/electron-browser/workingCopyBackupTracker.ts`.

### 8.1 Process model
One Electron main process owns every window (`windowsMainService`). Each window is a renderer with **its own extension host**, so the extensions and language servers they start run once per window. The integrated terminal's pty lives in a shared "pty host" process, but a terminal belongs to one window.

### 8.2 Settings that choose the window
- `window.openFoldersInNewWindow` (`on` | `off` | `default`, default `default`, scope APPLICATION): `on` new window; `off` **replaces the most recently active window**; `default` = new window, unless chosen from inside the app (File menu/dialog), in which case it replaces. Ignored when `--new-window` / `--reuse-window` is given.
- `window.openFilesInNewWindow` (`on` | `off` | `default`, default `off`): `off` = the window that has the file's folder open, otherwise the most recently active window. `default` on macOS = same as `off` unless opened through the Dock or Finder; on Linux/Windows it is a new window unless opened from a dialog/menu or from VS Code's own terminal (`TERM_PROGRAM=vscode`). Logic: `shouldOpenNewWindow` (`windowsMainService.ts:1432`).
- `window.openWithoutArgumentsInNewWindow` (`on` | `off`, default `off` on macOS, `on` elsewhere): what a second launch with no arguments does (`launchMainService.ts:~171`): a new empty window or focus the most recently active window.
- `window.restoreWindows` (`preserve` | `all` | `folders` | `one` | `none`, default `all`): `preserve` reopens every window *and* still opens folders from the CLI as extra windows; `all` reopens every window unless the command line passes a folder/workspace/file; `folders` only windows that have a folder; `one` only the most recently active window; `none` one empty window. After an update it always behaves like `all` (`getRestoreWindowsSetting`, `windowsMainService.ts:1134`).
- `window.closeWhenEmpty` (default false): closing the last editor closes the window, only for windows with no folder.
- `window.newWindowDimensions` (`default` | `inherit` | `offset` | `maximized` | `fullscreen`), `window.restoreFullscreen`, `window.nativeTabs` (native macOS tabs, needs a restart), `window.confirmBeforeClose` (`always` | `keyboardOnly` | `never`, desktop default `never`).

### 8.3 Finding a window for a path
`windowsFinder.ts`:
- `findWindowOnFile(windows, fileUri)`: first multi-root workspaces that have a folder containing the file, then single-folder windows whose folder is a parent of the file, **the deepest folder wins** (sorted by path length).
- `findWindowOnWorkspaceOrFolder`: exact match on a folder URI or workspace file; used so that opening an already open folder focuses that window.
- In `doOpen` (`windowsMainService.ts:~620`): files from the CLI/Dock/desktop/links use `findWindowOnFile`, otherwise the most recently active window; if that window has a folder, the folder is "reopened" (focused) along with the files; if it is empty, the files go straight into it; if there is no window, a new one opens.
- Passing several folders on the CLI makes one untitled multi-root workspace.
- A window being "replaced" (opening a folder with `openFoldersInNewWindow: off`): the renderer is unloaded with `ShutdownReason.LOAD` (dirty check / hot exit) and the new folder is loaded in the same native window.

### 8.4 The `code <path>` CLI
- `code` runs `cli.ts` under Electron-as-Node. On macOS it starts the app with `open -a <app> --args …` (env passed through `--env`); elsewhere it spawns the binary.
- The new main process first tries to **claim the main IPC socket** (`environmentMainService.mainIPCHandle`, Unix socket / named pipe; `main.ts:352 claimInstance`). `EADDRINUSE` → connect as a client, call `launch.start(args, env)` on the running instance, and exit ("Sent env to running instance. Terminating..."). If the connection fails because of a stale socket, it unlinks it and retries once.
- `launchMainService.start` maps the flags: `-n/--new-window` → `forceNewWindow`, `-r/--reuse-window` → `forceReuseWindow`, `-a/--add` → `addMode` (add the folder to the most recently active window; ignored when there is no window or at startup), `--remove`, `-g/--goto file:line[:col]` → `gotoLineMode`, `-d/--diff`, `--merge`. `preferNewWindow = !reuse && !wait`.
- `-w/--wait`: the CLI creates a temporary "wait marker" file and passes `--waitMarkerFilePath`; the app deletes it when the editor/window closes; the CLI polls `whenDeleted`. The second instance itself also waits on the used window's `whenClosedOrLoaded`.
- The shell's environment is forwarded (`userEnv`) and used for the window (terminal, tasks).

### 8.5 Last window on macOS, Dock, new window
- `lifecycleMainService.ts:300`: `window-all-closed` only quits if a quit was requested or the OS is not macOS. So on macOS the app keeps running with just the menu bar.
- `app.on('activate', (e, hasVisibleWindows))` (`app.ts:562`): no visible window → `openEmptyWindow({ context: DOCK })`. It does **not** restore the last session (restore only happens at startup).
- `app.on('new-window-for-tab')` → empty window (the macOS tab "+" button).
- `workbench.action.newWindow` (cmd+shift+n / ctrl+shift+n) opens an empty window (`forceNewWindow`, `forceEmpty`); it shows the Welcome page according to `workbench.startupEditor`. The new window inherits the profile of the most recently active window.
- Window bounds: `windowsStateHandler.ts` saves `windowsState` = `{ lastActiveWindow, openedWindows[] }` with uiState (bounds, mode, display) per folder/workspace/empty backup; a reopened folder has its own bounds, a new window cascades from the most recently active window (`ensureNoOverlap`). On macOS the state of the window closed last is kept in memory for the next window.

### 8.6 Quitting with dirty editors
- Quit: Electron `before-quit` → each window is unloaded with `ShutdownReason.QUIT`; any window can veto, which cancels the whole quit.
- `files.hotExit` (`off` | `onExit` default | `onExitAndWindowClose`): on QUIT everything dirty is backed up, **no prompt**, restored next run (`workingCopyBackupTracker.ts:217`). On window close (CLOSE) with `onExit`: macOS or more than one window → prompt (a window with a folder only backs up the scratchpad automatically); the last window on Win/Linux → backup, because the app quits right after. `onExitAndWindowClose` → also backs up a window with a folder when it closes.
- With `hotExit: off`, each window prompts in turn during quit.

### 8.7 Workspace trust, settings
- Trust is by folder URI (or parent folder), stored once in application storage and shared by every window; each window evaluates trust for its own folders and reacts to changes made in another window. An empty window follows `security.workspace.trust.emptyWindow`.
- Settings: user settings (per profile) are shared by every window; workspace settings (`.vscode/settings.json` or a `.code-workspace` file) are per window; folder settings are per root in multi-root. Setting scopes: `application` (only from user settings, read by the main process, for example every `window.open*`/`restoreWindows`), `machine`, `window`, `resource`, `language-overridable`.

## 9. Similarities and differences

| Topic | VS Code | Zed (b5cd310) |
|---|---|---|
| Unit per window | 1 workspace (folder or multi-root) | `MultiWorkspace`: N projects + sidebar |
| Default open folder | new window; from the menu, replaces the current window | add to the current window (`existing_window`) |
| Open file | the window that contains it (deepest folder), otherwise the most recently active window | the workspace that contains it, otherwise the active window |
| Open an already open folder | focus that window | focus that workspace (MatchExact) |
| Reuse an empty window | an empty window receives files | an empty workspace downgrades NewWindow→Activate |
| CLI channel | main IPC socket claimed by the first instance | LaunchServices URL (mac) / unix datagram (Linux) + ipc-channel back channel |
| CLI flags | `-n -r -a -g -w --remove` | `-n -a -e -w` (+ hidden `-r --classic`) |
| Restore | `restoreWindows` all/preserve/folders/one/none | `restore_on_startup` last_session/last_workspace/empty_tab/launchpad |
| Last window (mac) | app keeps running; Dock → empty window | app keeps running; Dock → restore session (or create new) |
| Last window (Linux/Win) | quit | quit (`on_last_window_closed`) |
| Quit with dirty buffers | hot exit by default, no prompt | hot exit into DB, no prompt; active window first |
| Language server | per window (extension host) | per project (≈ per window) |
| Terminal | per window, shared pty host | per project |
| Project settings | `.vscode/settings.json` per window | `.zed/settings.json` per worktree directory, one global store keyed by worktree |
| Theme/keymap | user level (theme can be a workspace setting) | global only |
| Trust | by folder, global store | by worktree path, global store |

## 10. What Lucerna needs to watch

Lucerna today (read-only look at `crates/lucerna-workbench/src/app.rs`, `app_state.rs`, `process.rs`): `Router { shown: Option<…> }` assumes one window; `start()` quits when the last window closes; `open_requests::route` can return `Route::NewProcess` and `process::spawn` starts another `luc`; `AppState.settings` is one `SettingsStore` with exactly one `workspace` layer.

1. **Settings layering is the biggest trap.** `AppState.settings` mixes the user layer and *one* workspace layer at global level. With two windows on two folders, the workspace layer must be per window (or keyed by root as Zed does with `(WorktreeId, path)`). Every `settings.get(..)` call then needs a location (window/root or file path). By VS Code's scopes: the settings that control windows (`window.*`, restore, open behavior) are application scope, read only from user settings.
2. **Behavior follows VS Code, mechanism follows GPUI**: implement `window.openFoldersInNewWindow`, `window.openFilesInNewWindow`, `window.restoreWindows`, `window.closeWhenEmpty`, `window.openWithoutArgumentsInNewWindow`, the `files.hotExit` semantics; use VS Code's finder rules (deepest folder wins; an already open folder gets focused). Copy Zed's "reuse the empty window" rule.
3. **Every open goes through one function** (like `open_paths`) that takes an options struct and returns the window that was used; Finder, Dock, CLI, menu, the recent list and restore all call it. Keep it a plain `fn(&mut App)` so it is testable: `TestPlatform::on_open_urls`/`on_reopen` panic.
4. **CLI → running app**: Lucerna already receives paths through `on_open_urls`. For `luc <path>` with flags (`-n`, `-r`, `-a`, `-g`, `-w`), the CLI needs a channel that carries flags and env, and an answer for `--wait`. Options: a VS Code-style socket (the first instance claims `<data_dir>/luc.sock`; handle stale sockets) or a Zed-style URL with a one-shot back channel. Avoid Zed's TCP-port single-instance check; a Unix socket in the data directory is simpler and carries over to Linux. Watch for the cold-start race (Zed issue #61346): queue CLI requests until restore has finished opening the windows.
5. **Quit**: GPUI's `on_app_quit` cannot veto. Do the prompt/hot exit in the `Quit` action across all windows *before* `cx.quit()`, active window first, one at a time; Cancel cancels the whole quit. A system quit (logout, Dock Quit) only runs `on_app_quit`, so hot exit must also be safe there (write the backup synchronously within `SHUTDOWN_TIMEOUT`, or always keep the backup up to date). Language servers: `QuitShutdown` already tracks every `LanguageServers` entity; check that it is still correct with several windows.
6. **Closing the last window**: replace the hand-written `on_window_closed → quit` in `start()` with `QuitMode::Default` (macOS keeps running, elsewhere quits) or keep the observer but only outside macOS. Then implement `on_reopen` (Dock click with no windows left) and make menu actions work with no window (a global `cx.on_action` for Quit, Open, Open Recent, New Window, Settings).
7. **`active_window()` can be `None`** when the app is in the background (for example a CLI run from Terminal). Track the "most recently active workbench window" yourself (like VS Code's `getLastActiveWindow`), falling back to `window_stack()`.
8. **Restore**: save the list of windows and their front-to-back order at quit (`window_stack`), remove a window from the session when the user closes it, restore in reverse order so the frontmost window ends up on top, and keep bounds + display per folder. A decision is needed on Dock reopen behavior: VS Code opens an empty window, Zed restores the session.
9. **Same file in two windows**: two editors have separate buffers (both Zed and VS Code do this across windows). Lucerna needs reload-on-disk-change and conflict handling that work correctly when the other window saves; consider focusing the window that already has the file (VS Code's finder) so this case is rare.
10. **Per-window and shared resources**: per-window language servers match both editors and keep things simple; sharing one server across windows needs per-server workspace-folder support. Keep `LanguageRegistry`, theme, icon theme, extensions and keymap as app globals. The trust store is global, evaluated against each window's roots; changing trust in one window must update the other windows.
11. **Globals that assume one window**: review the `impl Global` types (`StatusMessage`, `SystemShellGlobal`, `Spawner`, `DefaultTerminalSpawner`, `QuitShutdown`, `TestRouter`) and the `Router.shown` field. In both editors, the status message in particular is per window.
12. **macOS native tabs**: GPUI only enables them when `WindowOptions.tabbing_identifier` is `Some`; leave it `None` (VS Code's `window.nativeTabs` defaults to false) unless a setting asks for it.
13. **Tests**: use `TestAppContext::add_window` for several windows, `cx.windows()`, `VisualTestContext::simulate_close()`, `simulate_prompt_answer` for the quit-with-dirty flow. `window_stack()` is `None` in tests, so the code must not rely on it to be correct.
14. **Keep platform-specific code separate**: the socket path, LaunchServices, Dock reopen and "quit on closing the last window" differ by OS; put them behind small functions so Linux/Windows can follow.

## 11. What Lucerna chooses

- One `luc` process for every window; one Dock icon.
- Theme, icon theme, keymap: shared by the whole app, read only from user settings (following Zed; deviating from VS Code, which lets `workbench.colorTheme` be set in a workspace). Reason: keeps the ~140 color-reading sites unchanged, and every window looks the same.
- Settings: the default and user layers are shared; the workspace layer (`.lucerna/settings.json`) is per window, a separate entity that each `Workbench` owns and passes down to its child entities (following VS Code: each window has its own `ConfigurationService`). Not Zed's location-keyed approach, because Lucerna has only one settings file at the root.
- Closing the last window: on macOS the app keeps running, and a Dock click opens Welcome (following VS Code); Linux/Windows quit.
- `luc <path>`: a Unix socket in the data directory (following VS Code), not LaunchServices.
- `window.restoreWindows` defaults to `"all"`, like VS Code.
- Opening a folder that is already open in another window focuses that window; a file opens in the window whose folder contains it most deeply (`findWindowOnFile`).
- Language server, terminal: per window, like both references. Trust: one shared store, evaluated against each window's roots; a change in one window updates the others.
