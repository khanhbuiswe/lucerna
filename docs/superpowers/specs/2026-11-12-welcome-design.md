# Welcome screen, Open Recent and menu bar

Date 2026-11-12. Status: implemented (commits 336a5e5–a03b07b). The manual check on the real app (§12) is not done yet. The menu bar part (W11, W13, W14, §9) was folded in at the user's additional request.

> **Partly superseded** by the multi-window spec
> (`2026-11-15-multi-window-design.md`): every Lucerna window now runs in
> a single `luc` process. Wherever this spec says "one process per window",
> "start `luc <path>` in a new process" or "several processes write at once"
> (W1, W8, the not-doing part of §1, writing `recent.json`, the `open_requests.rs` row,
> error reporting), follow the multi-window spec: "New Window" opens a window in the same
> process through `router::open`, and `process.rs`/`spawn_process` are gone.

Where the implementation differs from the spec:
- **Menu table (§9.1):** two extra variants, `Entry::Submenu` (for Settings ▸) and `Entry::Only(Platform, …)` (for Open…, which exists only on macOS).
- **Recent entries in File ▸ Open Recent** run their own action `OpenRecentEntry`.
- **Items the current window cannot run** use the action `DisabledCommand`, because this action has no listener, so AppKit shows it disabled.
- **A command invoked from the menu only runs where its shortcut would run** (checked against the keymap's `when`). Example: Edit ▸ Undo does nothing while the search box has focus.
- **Open Recent drops the entries of the currently open window itself**, both in the menu and in the picker. A window that opened only a file drops both that file and its parent directory.
- **The logo on Welcome** uses `mark-mono.svg`, tinted with the theme's `text` color.
- **In the picker**, when filtering leaves no items, it shows "No matching results".
- **Opening a File-kind recent entry from the picker in the workbench** opens it as a pinned tab in that same window.
- **File writes:** the atomic write uses a temp file per process (`<name>.<pid>.tmp`). `record` and `remove` do not write when reading the file failed for a reason other than "does not exist".

## 1. Goals

Opening Lucerna from the Dock or Finder without a folder shows the **Welcome screen** instead of the folder-picking dialog, where pressing Cancel quits the app. From Welcome and from inside a window that has a folder open, the user reopens a recent folder with **Open Recent** (⌃R), as in VS Code. When no tabs remain, the editor area hints at a few shortcuts.

Current state:
- `app.rs` `pick_folder` shows the dialog right after a 300 ms wait for an open request from Finder.
- The app has no list of recent folders yet. `RecentFiles` only serves Quick Open inside one project.
- `editor_groups/view.rs` `empty_hint` already shows "Lucerna", "Go to file ⌘P" and "Find in project ⇧⌘F".

Decisions (W1–W4 are the scope the user agreed to; W5–W12 are proposed by research and need approval):

| # | Decision | Reason |
|---|---|---|
| W1 | Welcome is the **own root view** of a window with no folder (`WelcomeView`), made of a title bar and one centered content column. Choosing a folder swaps the window's root to a `Workbench` (`window.replace_root`), without opening a new window. | `Workbench` always needs a root, and the workspace config is global, tied to one root. Zed draws Welcome in a workspace with no worktree, while Lucerna has no empty workbench. Swapping the root within the window avoids flicker. *(Note 2026-11-16: "the workspace config is global" is no longer true; each window has its own `WindowSettings`, see the multi-window spec §6.1. Welcome is still its own root view.)* |
| W2 | Command `workbench.action.openRecent`, key `ctrl+r`. The picker reuses the existing picker frame (`popover`, `picker_input_row`, `picker_list`, `picker_row`, `fuzzy_filter`). | Command and key follow VS Code; the UI is shared with Quick Open and the theme picker. |
| W3 | The empty editor area shows a watermark of commands and keys, with labels taken from the user's keymap. Toggled by `workbench.tips.enabled`. | VS Code does this. Zed leaves the pane empty when there is a folder; Lucerna departs from Zed here because users coming from VS Code need to see the keys. The watermark already exists, it is only extended. |
| W4 | Setting `workbench.startupEditor`, default `"welcomePage"`. | Follows VS Code (§5). |
| W5 | The list is stored at `<data_dir>/recent.json`, at most 100 entries, newest first. A path that no longer exists is **hidden when read** (like Zed) and **dropped from the file on the next write**. | "Drops itself" exactly as requested. The existence check runs on a background thread. |
| W6 | The list records at the **window level**: a folder opened as a window, or a file opened on its own (`single_file`). A file opened as a tab inside a folder is not recorded (Quick Open already has `RecentFiles` for that). | VS Code records files too. Lucerna only lets the Welcome screen and Open Recent take the user back to the right window; the list of files in a project already lives elsewhere. |
| W7 | In the picker, Enter opens **replacing the current window**, ⌘Enter opens a **new window**. On Welcome, a click opens in that window, and holding ⌘ while clicking opens a new window. | Follows VS Code. |
| W8 | "Replace the current window" while a folder is open: ask to save as on window close (`confirm_quit`), save the layout, then **swap this window's own root** to the new folder's `Workbench` (`window.replace_root`), as in W1. "New window" starts `luc <path>` in a new process. *(Fixed 2026-11-13; the first version started a new process and then closed the window, which made the window reopen with a zoom-in animation.)* *(Superseded by `2026-11-15-multi-window-design.md`: "New window" opens a window in the same process, and starts no process.)* | VS Code (File > Open Folder, `window.openFoldersInNewWindow: "default"`) and Zed (File > Open) both switch folders in the current window. `Workbench` emits `WorkbenchEvent::Replace`; `Router` listens and swaps the root, updating `watch_config`, the recent list and the menu bar. The old `Workbench` is released: `on_release` shuts down LSP in the background, `Worktree` stops its watcher on drop, and other views only hold a `WeakEntity`. A test checks that the old `Workbench` can no longer be upgraded. |
| W9 | On macOS, Welcome has "Open…" (⌘O, `workbench.action.files.openFileFolder`, a dialog that can pick both files and folders) and "Open Recent…" (⌃R). On Linux and Windows, "Open…" is replaced by "Open File…" and "Open Folder…". | Follows VS Code on each platform. Platform-specific keys and labels live in `cfg`, so no macOS-only assumption is added elsewhere. |
| W10 | Add `workbench.action.closeFolder` (`cmd+k f`): ask to save, then **swap this window's root to Welcome**, re-reading the user-only config (`ConfigPaths::user_only`). *(Fixed 2026-11-13, same reason as W8.)* | VS Code has this command and stays in that window. Having Welcome without a way back would be incomplete. |
| W11 | **Full menu bar** (§9): Lucerna, File, Edit, Selection, View, Go, Window, Help. Menu items are declared once in a table of VS Code-style command IDs; an item whose ID is not registered is hidden. The key next to each item follows the user's current keymap. | The user's additional request. Both VS Code and Zed build menus from the editor's own commands. |
| W12 | No AI content, sign-in, onboarding, New File or Clone Repository. | Lucerna does not have those features. Untitled files do not exist yet. |
| W13 | Menus call commands through a single GPUI action `RunCommand { id }`. Displayed keys are registered as GPUI `KeyBinding`s in the context `LucernaMenu`. No element has this context, so keys are still handled by `keys.rs`. | GPUI only reads menu keys from the GPUI keymap, while Lucerna commands are string IDs. A dedicated context makes the binding display-only, so the command does not run twice. |
| W14 | Add `workbench.action.showAboutDialog` ("About Lucerna"), using `window.prompt` to show the version, with a Copy button. | The app menu needs About. VS Code has this command; Lucerna did not. |

**Success when:**
- Opening `Lucerna.app` from the Dock: Welcome shows with the list of recent folders. Clicking an entry turns the window into that folder's workbench, with no second window.
- ⌃R on Welcome or inside the workbench opens the picker. Typing filters. Enter opens the folder, ⌘Enter opens a new window. Clicking × removes the entry from the picker and from `recent.json`.
- Deleting (or renaming) a previously opened folder: next time it is no longer in Welcome or the picker, and no longer in `recent.json` after the next write.
- Closing all tabs: the editor area shows the watermark with keys that match the user's keymap. Setting `"workbench.tips.enabled": false` removes the watermark.
- `"workbench.startupEditor": "none"` starts as it does today (the folder-picking dialog).
- The menu bar has all 8 menus. The key next to each item changes when the user edits `keybindings.json`. The Edit menu has Emoji & Symbols and Start Dictation added by macOS. The Help menu has macOS's command search box. Items for commands that do not exist are not shown.
- All old tests stay green. `cargo clippy`, `cargo fmt --check` and `cargo deny check` are all clean.

**Out of scope:**
- A Welcome page as a tab in a window with a folder (VS Code's `startupEditor: "welcomePage"` opens Welcome even when no tab is restored; Lucerna lets the watermark do this).
- The values `readme`, `newUntitledFile`, `terminal`.
- A Dock menu of recents (`NSApplication` dock menu).
- Multi-folder workspaces; remote.
- Syncing the list between running processes: each process reads the file when needed, so it always sees the latest version.

## 2. References

Details are in `docs/research/2026-11-12-welcome.md`. Summary:
- **VS Code:**
  - Welcome is a tab, with Start (Open… / Open File… / Open Folder…) and Recent (5 folders, an × button on hover, a More… link).
  - `workbench.startupEditor` has `none | welcomePage | readme | newUntitledFile | welcomePageInEmptyWorkbench | terminal`.
  - `workbench.action.openRecent` uses `ctrl+r`; the picker is split into "folders" and "files" groups, with an × button; Enter opens in the same window, ⌘Enter opens a new window.
  - The list is JSON in application-level storage, at most 500 entries; missing paths are kept.
  - The watermark is toggled by `workbench.tips.enabled`.
- **Zed:**
  - `WelcomePage` is drawn in the empty pane of a workspace with no folder. The column is at most 512 px wide, centered. It has a logo and a title; section headers are UPPERCASE, XSmall, muted, with a rule.
  - Buttons span the whole column, with a muted icon, a label, and the key on the right. There are 5 recent projects, and `cmd-1`…`cmd-5` open each one.
  - The `OpenRecent` picker uses `ctrl-r`; the button at the end of a row only shows on hover.
  - Missing paths are hidden when read. An empty pane in a window with a folder stays empty.
- **Menu bar:**
  - VS Code builds menus from `MenuId.Menubar*Menu`, including Code, File, Edit, Selection, View, Go, Run, Terminal, Window, Help.
  - Zed declares menus in `app_menus.rs` (Zed, File, Edit, Selection, View, Go, Run, Window, Help), uses `os_action` for Edit, and rebuilds menus when the keymap changes.
  - Displayed keys come from the GPUI keymap. On Linux/Windows, Zed draws the menu in the title bar.
- **Lucerna:** commands, keys and settings follow VS Code; the Welcome layout, the × button shown on hover and `cmd-1`…`cmd-5` follow Zed; the watermark stays (a departure from Zed, see W3).

## 3. The recent list (`recent.rs`)

A pure module, no gpui, so it can be unit tested.

```rust
pub enum RecentKind { Folder, File }
pub struct RecentEntry { pub path: PathBuf, pub kind: RecentKind }
pub struct RecentList { entries: Vec<RecentEntry> } // newest first
```

The file `<data_dir>/recent.json` (`data_dir` is `default_data_dir()`, the same place as `workspaces/`):

```json
{ "version": 1, "entries": [ { "path": "/Users/me/code/lucerna", "kind": "folder" } ] }
```

Operations:
- `load(path) -> RecentList`:
  - File missing: an empty list.
  - File corrupt or a different version: rename it to `.bak` as `load_state` does, then return an empty list.
  - Missing paths are not filtered at this step.
- `existing(&self) -> Vec<RecentEntry>`: keeps only entries whose `path` still exists and is of the right kind (a directory for `Folder`, a file for `File`). This function calls `stat`, so the UI always runs it on `background_executor`.
- `record(data_dir, entry)`, `remove(data_dir, path)`, `clear(data_dir)`: each re-reads the file, edits it, drops missing entries, then writes atomically. Specifically:
  - `record` removes an entry with the same `path`, puts the new entry first, and truncates to `RECENT_LIMIT = 100`.
  - The atomic write shares the write-temp-then-rename function with `save_state`. Split that function out as `write_atomic(path, bytes)` in `workspace_state.rs`.
  - These functions also run on a background thread.
- Paths are always canonical, because `Router::open_path` already canonicalizes.

Several `luc` processes writing the same file: each write re-reads right before writing, so data is only lost when two writes overlap within a few milliseconds. Acceptable for a suggestion list; no file lock is used.

**When to write:** `Router` calls `record` after `open_workbench` has opened a window (from the CLI, Finder, Welcome or the picker). There are two kinds of entries:
- `Folder(root)`, when the window opens a folder;
- `File(file)`, when the window opens a file on its own (`single_file`).

Opening `luc somefile` inside a git repo records the root folder, because the window belongs to that folder.

**Display:**
- The name is `file_name()`.
- The description is the parent path, with `~` abbreviation using the existing `path_bar_text` logic; factor that `~` logic out into a shared function.
- The folder open in the current window is not in the displayed list (like VS Code's Welcome page), so focus always starts on the first entry.

## 4. The Welcome screen (`welcome.rs`)

### 4.1 Layout (following Zed `welcome.rs`)

```
┌──────────────────────────────────────────────────────────────┐
│ ● ● ●                       Lucerna                          │  title bar 34 px (layout::title_bar)
├──────────────────────────────────────────────────────────────┤
│                                                              │
│                [logo 45px]  Welcome to Lucerna               │
│                             A minimal code editor            │  muted, small
│                                                              │
│                GET STARTED ───────────────────────────       │  section header
│                [📂] Open…                          ⌘O        │
│                [🕘] Open Recent…                   ⌃R        │
│                                                              │
│                RECENT ────────────────────────────────       │
│                [▢] lucerna    ~/Projects.nosync     ⌘1  ×    │  × only on hover
│                [▢] zed        ~/src                 ⌘2       │
│                [▢] notes.md   ~/Desktop             ⌘3       │  kind File: file icon
│                More…                                         │  only when > 5 entries
│                                                              │
└──────────────────────────────────────────────────────────────┘
```

- The background is `editor_background`, and the whole page has no status bar. The content column is at most 512 px wide, centered on both axes, with `8×GRID` padding, blocks `6×GRID` apart, scrolling vertically if the window is short.
- Page top:
  - the Lucerna logo (`assets/brand/mark.svg`) next to the title "Welcome to Lucerna", 20 px;
  - a subtitle line at `UI_FONT_SIZE_SMALL`, in `text_muted`.
  - When there are recent entries, the title is "Welcome back to Lucerna", as in Zed.
- **Section header** (a new component in `ui.rs`: `section_header(title, cx)`): an uppercase title at `UI_FONT_SIZE_CAPTION`, editor font, `text_muted`, followed by a 1 px `border_variant` rule filling the rest.
- **Row button** (a new component in `ui.rs`: `list_button(id, icon, label, keys, cx)`):
  - `7×GRID` tall (equal to `PICKER_ROW_HEIGHT`), full column width, `GRID` corner radius;
  - `ghost_element_hover` background on hover, `element_selected` background when focused;
  - a 14 px icon in `icon_muted`, the label at `UI_FONT_SIZE`, truncated if long;
  - the key on the right, at `UI_FONT_SIZE_STATUS`, in `text_muted`.
  - A recent row additionally has a description (the parent path, muted, truncated) and an × button (`icon_button` with `CLOSE_ICON`), shown only on hover or when the row has focus.
- The RECENT section has at most 5 rows. When the list is empty this section is not drawn (Zed also hides it). The "More…" row opens the picker.

### 4.2 Behavior

- **Keyboard:**
  - ↑/↓ and Tab/⇧Tab move focus between buttons; Enter presses the focused button; ⌘Enter opens a new window.
  - Initially focus is on the "Open…" button.
  - ⌘1…⌘5 open the 1st–5th recent entry (Zed).
- **Global keys** in the Welcome window go through the user's keymap (`AppState.keymap`), as the workbench does in `keys.rs`. Only the commands in §6 take effect; other keys are ignored.
- **Open… / Open File… / Open Folder…:** call `cx.prompt_for_paths`. Pressing Cancel stays on Welcome and does not quit the app. The chosen path goes through `Router::open_path`, like an open request from Finder.
- **Clicking a recent entry:**
  - If the path is gone (deleted after the list was loaded): remove the entry from the list and show a `warning_row` "‹path› no longer exists" at the top of the column.
  - Otherwise: open it like "Open…".
  - Clicking × calls `remove` and redraws.
- **Opening a folder from Welcome:**
  - `watch_config(ConfigPaths::for_root(root))`;
  - `window.replace_root(...)` to a `Workbench`, the window keeping its position and size;
  - `Router` updates the handle and `OpenWindow`;
  - `record`.
- **The list is loaded on a background thread** when the view is created: `load` then `existing`, then `cx.notify()`. Meanwhile the RECENT section is not drawn, so there is no flickering empty frame.
- **Quitting the app:** closing the Welcome window quits the app, like closing the last window today. ⌘Q still works.

### 4.3 Config when there is no folder

`watch_config` currently needs a root to read `<root>/.lucerna/settings.json`. Add `ConfigPaths::user_only()`, in which `workspace_settings: None`: the loader skips the workspace layer and the watcher only watches the user's directory. Welcome needs this config to get the theme, keymap and `startupEditor`. When switching to a folder, `watch_config` is called again with `for_root`, replacing the global `ConfigState` as at startup.

## 5. `workbench.startupEditor`

Add to `default-settings.json`:

```jsonc
// Controls which editor is shown at startup, if none are restored from the previous session.
// "welcomePage" | "welcomePageInEmptyWorkbench": the Welcome screen when Lucerna starts without a folder.
// "none": ask for a folder instead; Cancel quits.
"workbench.startupEditor": "welcomePage",
```

- `run(None, …)`: after the wait for an open request from Finder ends (300 ms, unchanged), the app reads the user's setting. `none` calls `pick_folder` as before; any other value opens the Welcome window.
- `welcomePage` and `welcomePageInEmptyWorkbench` give the same result, because Lucerna has no Welcome tab in a window with a folder. `readme`, `newUntitledFile` and `terminal` are not supported yet, so they are treated as `welcomePage`.
- The order of reading before opening the window: `start` → `watch_config(user_only)` → read the setting → open Welcome or show the dialog. Reading the setting means synchronously reading a small file before there is a window, just as `watch_config` already does at startup.

## 6. Commands, keys, menu

| Command | Title (Command Palette) | Default key | Where |
|---|---|---|---|
| `workbench.action.openRecent` | File: Open Recent… | `ctrl+r` | Welcome, workbench |
| `workbench.action.files.openFileFolder` | File: Open… | `cmd+o` (macOS) | Welcome, workbench |
| `workbench.action.files.openFile` | File: Open File… | (Linux/Windows `ctrl+o`, when there is a separate keymap for those platforms) | Welcome, workbench |
| `workbench.action.files.openFolder` | File: Open Folder… | (Linux/Windows `ctrl+k ctrl+o`) | Welcome, workbench |
| `workbench.action.clearRecentFiles` | File: Clear Recently Opened… | none | Welcome, workbench, menu |
| `workbench.action.closeFolder` | File: Close Folder | `cmd+k f` | workbench |

- **In the Open Recent picker** (when `inRecentPicker` is true):
  - `ctrl+r` selects the next entry, `ctrl+shift+r` the previous one;
  - `cmd+enter` opens a new window; `escape` closes the picker (using the existing `workbench.action.closeQuickOpen`).
- **Edits in shared files:**
  - `default-keybindings.json`: add 5 lines (`ctrl+r`, `cmd+o`, `cmd+k f`, and 2 navigation lines in the picker);
  - `default-settings.json`: add 2 keys (`workbench.startupEditor`, `workbench.tips.enabled`);
  - `lib.rs`: add 3 `mod` lines.
  - `cmd+k f` does not touch the existing `cmd+k` chords (`cmd+k cmd+t`, `cmd+k f12`, `cmd+k s`, `cmd+k cmd+i`).
- **Collision with Vim:**
  - Vim V1 does not use `ctrl+r`.
  - Vim V2 will use `<C-r>` for redo. `vim::dispatch` receives keys before the keymap when the editor has focus, so `ctrl+r` inside the editor will become redo, while outside the editor it still opens Open Recent. VSCodeVim does the same, so no `when` is needed.
- **Open… in the workbench:**
  - Choosing a file: opens it as a pinned tab in the current window, even a file outside the root (VS Code opens the file in the current window).
  - Choosing a folder: replaces the window, as in W8.
- **Clear Recently Opened:** asks again with `window.prompt` ("Do you want to clear all recently opened folders?", buttons Clear / Cancel).
- **Menu:** these commands live in the File menu (§9).

## 7. The Open Recent picker (`recent_picker.rs`)

`RecentPicker` is an entity like `ThemePicker`. It is shown in `popover_overlay`, on both Welcome and the workbench. The workbench holds it like `theme_picker`: it is added to `other_modal_overlay_shown` and to the places that check `theme_picker.is_some()` (keys, focus, `inQuickOpen`) so that it behaves like the other pickers.

```
┌──────────────────────────────────────────────────────┐
│ Select to open (⌘Enter for a new window)             │  picker_input_row, placeholder
├──────────────────────────────────────────────────────┤
│ FOLDERS                                              │  subheader, only when files are also present
│ ▢ lucerna   ~/Projects.nosync                        │  selected row: element_selected
│ ▢ zed       ~/src                         ⧉  ×       │  hover: "Open in New Window" button and ×
│ FILES                                                │
│ ▤ notes.md  ~/Desktop                                │
└──────────────────────────────────────────────────────┘
```

- Each row is a `picker_row` with a folder or file icon, the name with matched characters highlighted (`highlighted`), and the parent path in muted color.
- Fuzzy filtering runs on the string `"{name} {parent}"`, so typing the parent folder name also matches (VS Code `matchOnDescription`). While filtering, group headers are dropped and rows are ordered by score, as in Quick Open.
- Two buttons only show on hover:
  - "Open in New Window" (an arrow-out icon, with ⌘Enter in the tooltip);
  - × "Remove from Recently Opened", which calls `remove` then removes the row while the picker stays open.
- Empty list: one muted line "No recently opened folders".
- A path that is gone at selection time: handled as on Welcome. In the workbench, the message goes through `status_message`.

## 8. Watermark of the empty editor area

Rewrite `empty_hint` in `editor_groups/view.rs`, keeping the debug selector `editor-empty-hint`.

- Remove the big "Lucerna" text.
- Replace it with a centered two-column grid: labels right-aligned, keys left-aligned, `3×GRID` apart, text at `UI_FONT_SIZE`, in `text_muted`. Each row is:
  - Show All Commands — `workbench.action.showCommands`;
  - Go to File — `workbench.action.quickOpen`;
  - Find in Files — `workbench.action.findInFiles`;
  - Toggle Sidebar — `workbench.action.toggleSidebarVisibility`;
  - Open Recent — `workbench.action.openRecent`.
- A command that has no key in the user's keymap loses its row, as today.
- Add the setting `"workbench.tips.enabled": true`. When it is `false`, the empty area is just the background.

## 9. Menu bar (`app_menu.rs`)

Research is in `docs/research/2026-11-12-welcome.md` §4. Currently `install_app_menu` only has Lucerna (Hide, Quit) and Window (Minimize).

### 9.1 The menu table

The menu table is static data, declared once and platform-independent:

```rust
enum Entry {
    Command { title: &'static str, id: &'static str, os: Option<OsAction> },
    Separator,
    RecentSubmenu,           // File ▸ Open Recent
    Services,                // only built on macOS
    App(AppAction),          // Hide, HideOthers, ShowAll, Minimize, Zoom: GPUI actions, not commands
}
struct MenuSpec { title: &'static str, role: Role, entries: &'static [Entry] }
enum Role { App, File, Edit, Selection, View, Go, Window, Help }
```

A `Command` entry whose `id` is not registered in `CommandRegistry` is dropped when building the menu. A separator that ends up first, last or adjacent to another after dropping entries is also merged or dropped. This way the 4.2 commands appear by themselves when that branch is merged into `main` later.

The list of commands comes from `Workbench::command_ids()`. This function builds the default registry exactly as `Workbench::new` does (the `register_commands` functions), without needing a window.

| Menu | Items (command ID; ★ is a 4.2 command, hidden until it exists; ✚ is a new command of this spec) |
|---|---|
| **Lucerna** | About Lucerna (`workbench.action.showAboutDialog` ✚) · — · Settings ▸ { Open Settings (JSON) `workbench.action.openSettingsJson`, Open Workspace Settings (JSON) `workbench.action.openWorkspaceSettingsFile`, Open Keyboard Shortcuts (JSON) `workbench.action.openGlobalKeybindingsFile`, —, Color Theme… `workbench.action.selectTheme`, File Icon Theme… `workbench.action.selectIconTheme` } · Extensions `workbench.view.extensions` · — · Services · — · Hide Lucerna · Hide Others · Show All · — · Quit Lucerna `workbench.action.quit` |
| **File** | Open… `workbench.action.files.openFileFolder` ✚ (macOS only) · Open File… `…files.openFile` ✚ and Open Folder… `…files.openFolder` ✚ (outside macOS) · Open Recent ▸ (first 10 entries, —, More… `workbench.action.openRecent` ✚, Clear Recently Opened… `workbench.action.clearRecentFiles` ✚) · — · Save `workbench.action.files.save` · Save All `…files.saveAll` · Save without Formatting `…files.saveWithoutFormatting` · Revert File `…files.revert` · — · Close Editor `workbench.action.closeActiveEditor` · Close Folder `workbench.action.closeFolder` ✚ |
| **Edit** | Undo `undo` (`OsAction::Undo`) · Redo `redo` (`OsAction::Redo`) · — · Cut `editor.action.clipboardCutAction` (`OsAction::Cut`) · Copy `…clipboardCopyAction` (`OsAction::Copy`) · Paste `…clipboardPasteAction` (`OsAction::Paste`) · — · Find `actions.find` · Find in Files `workbench.action.findInFiles` · — · Toggle Line Comment `editor.action.commentLine` ★ |
| **Selection** | Select All `editor.action.selectAll` (`OsAction::SelectAll`) · Expand Selection `editor.action.smartSelect.expand` ★ · Shrink Selection `editor.action.smartSelect.shrink` ★ · — · Copy Line Up `editor.action.copyLinesUpAction` ★ · Copy Line Down `…copyLinesDownAction` ★ · Move Line Up `editor.action.moveLinesUpAction` ★ · Move Line Down `…moveLinesDownAction` ★ |
| **View** | Command Palette… `workbench.action.showCommands` · — · Explorer `workbench.view.explorer` · Search `workbench.view.search` · Extensions `workbench.view.extensions` · — · Toggle Sidebar `workbench.action.toggleSidebarVisibility` · Split Editor `workbench.action.splitEditor` · — · Word Wrap `editor.action.toggleWordWrap` · Zoom In `editor.action.fontZoomIn` · Zoom Out `editor.action.fontZoomOut` · — · Language Server Health `lucerna.lsp.health` · Language Server Log `lucerna.lsp.showLog` |
| **Go** | Back `workbench.action.navigateBack` · Forward `workbench.action.navigateForward` · — · Go to File… `workbench.action.quickOpen` · Go to Symbol in Workspace… `workbench.action.showAllSymbols` · Go to Symbol in Editor… `workbench.action.gotoSymbol` · Go to Line… `workbench.action.gotoLine` · — · Go to Definition `editor.action.revealDefinition` · Go to Type Definition `editor.action.goToTypeDefinition` · Go to Implementations `editor.action.goToImplementation` · Go to References `editor.action.goToReferences` · — · Next Problem `editor.action.marker.nextInFiles` · Previous Problem `editor.action.marker.prevInFiles` |
| **Window** | Minimize · Zoom (`window.zoom_window()`). macOS adds the window list to this menu by itself (GPUI calls `setWindowsMenu` for the menu named "Window"). |
| **Help** | Show All Commands `workbench.action.showCommands` · Open Logs Folder `lucerna.openLogsFolder` |

Item titles follow VS Code's menu titles (without the group prefix such as "File:" of the Command Palette).

### 9.2 Building the menu (`build_menus`)

`build_menus(spec, &BuildInput) -> Vec<gpui::Menu>` is a pure function, testable without a window. `BuildInput` contains:
- the list of registered command IDs;
- the `Platform`;
- the 10 recent entries;
- the kind of the active window (`Welcome` or `Workbench`).

This function does the following:
- **Per platform** (only in this build step; the data table is unchanged):
  - **macOS:** `Role::App` becomes the first menu, named "Lucerna".
  - **Linux/Windows** (following VS Code):
    - no app menu;
    - About moves to the end of Help;
    - Settings ▸ becomes File ▸ Preferences;
    - Quit becomes "Exit", at the end of File;
    - drop Services, Hide, Hide Others and Show All.
- **In the Welcome window:** only enable the items Welcome can handle: About, Open…, Open File…, Open Folder…, Open Recent, Clear Recently Opened, Quit, Hide…, Minimize, Zoom. The other items, including Settings ▸ (opening a settings file needs a workbench), are built with `.disabled(true)`, as VS Code does when the window is empty.
- **Each `Command` entry** becomes `MenuItem::action(title, RunCommand { id })`, or `MenuItem::os_action(…, os)` when `os` is set.

### 9.3 Running commands and showing keys

- `RunCommand { id: SharedString }` is a GPUI action (derive `Action`, namespace `lucerna`). It has one global listener (`cx.on_action`) that forwards the command to the active window:
  - Root is `Workbench`: `run_command(id, None, …)`.
  - Root is `WelcomeView`: Welcome's commands.
  - No window: ignore.
- **Displayed keys** (W13): each time the menu is built, for each ID, take the first **single-key** binding (not a chord) in the Lucerna keymap whose `when` is empty or true in the focused-editor context. That binding is turned into `KeyBinding::new(keystroke, RunCommand { id }, Some("LucernaMenu"))`.
  - Before re-binding, call `cx.clear_key_bindings()`. Then re-bind the fixed GPUI keys (Hide ⌘H, Hide Others ⌥⌘H, Minimize ⌘M, Quit ⌘Q, shown in `install_app_menu`), then bind the newly changed keys. Lucerna currently only uses `bind_keys` in `app.rs`.
  - GPUI picks the binding matching the Workspace/Pane/Editor context first, and if none matches takes the first binding (`find_or_first`). Because `LucernaMenu` does not match, the menu still shows the right key. No element has the key context `LucernaMenu`, so GPUI does not run the binding by itself; keys still go through `keys.rs`.
  - A chord (`cmd+k f`) cannot be shown in the macOS menu (GPUI only shows single-key bindings), same as Zed.
- **Key equivalents:** the GPUI window receives keys before the menu (`handle_key_equivalent`). When Lucerna handles the key the menu no longer runs, so the command is not run twice. If the window does not handle it (for example it has no focus), the menu runs `RunCommand`, with an identical result.
- **Rebuild the menu** (`cx.set_menus`) when:
  - the keymap changes (observe `ConfigState.keymap_generation`);
  - the recent list changes within the process (`record`, `remove`, `clear`);
  - the window's root changes (Welcome to Workbench).
  
  The recent list for the menu is loaded on a background thread before `set_menus` is called.

### 9.4 macOS-specific items

- **Edit:** the title must be exactly "Edit" and have Cut/Copy/Paste/Undo/Redo as `os_action`. AppKit recognizes this menu as the Edit menu and adds "Emoji & Symbols" and "Start Dictation…" itself. Input fields in system dialogs (Open, Save) can also use ⌘X, ⌘C, ⌘V thanks to the selectors `cut:`, `copy:`, `paste:`.
- **Help:** the title must be exactly "Help". AppKit attaches a search box (searching by menu item name) to this menu by itself. GPUI does not call `setHelpMenu`. If the search box does not show on a trial run, the fix is to call `setHelpMenu` in Lucerna's native layer (`#[cfg(target_os = "macos")]`, after `set_menus`). The plan will include a step to check this.
- **Services** uses `MenuItem::os_submenu("Services", SystemMenuType::Services)`.

### 9.5 Linux and Windows

The data table and `build_menus` already support both platforms (§9.2), but **drawing the menu in the title bar is left for later**. When done: `title_bar` reads `cx.get_menus()` and draws each menu as a dropdown button, like Zed's `ApplicationMenu`. This spec does not do that drawing.

## 10. Code architecture

| File | Change |
|---|---|
| `recent.rs` (new) | `RecentList`, `RecentEntry`, `load`, `existing`, `record`, `remove`, `clear`, `RECENT_LIMIT`. No gpui. |
| `welcome.rs` (new) | `WelcomeView`: render (§4), key handling, loading the list on a background thread, showing `RecentPicker`. |
| `recent_picker.rs` (new) | `RecentPicker` and `RecentPickerEvent::{Open { entry, new_window }, Dismissed}`. |
| `ui.rs` | Add `section_header`, `list_button`. |
| `app.rs` | Change `Router.window` to the enum `Shown::{Welcome(handle), Workbench(OpenWindow, handle)}`. Add `Router::open_welcome`, `Router::open_in_welcome(path)` (swap root), call `record` after opening, read `startupEditor`, call `app_menu::install`. Router is `Rc<RefCell<…>>`, stored as a global (`RouterHandle`) so the workbench can call it. |
| `open_requests.rs` | `spawn_process` returns `io::Result<()>` so W8 does not close the window when starting the new process fails. Add `spawn_welcome_process()` (`cwd = /`, no arguments). |
| `workbench.rs` | Register the commands of §6 through the new module's `register_commands` (like `theme_picker::register_commands`). Add `Workbench::command_ids()` (§9.1). Add `replace_with(path)` for W8: `confirm_quit` → `save_workspace_state` → spawn → `window.remove_window()`. |
| `config_files.rs` | `ConfigPaths.workspace_settings: Option<PathBuf>` and `ConfigPaths::user_only()`. |
| `workspace_state.rs` | Split out `write_atomic`; add the shared function that abbreviates `~`. |
| `editor_groups/view.rs` | `empty_hint` per §8. |
| `app_menu.rs` (new) | The menu table (§9.1), `build_menus` (pure), `RunCommand`, the global listener, registering displayed keys, rebuilding the menu. `install_app_menu` in `app.rs` moves here. |
| `about.rs` (new, small) | The command `workbench.action.showAboutDialog` (W14). |
| `default-settings.json`, `default-keybindings.json` | Additions as in §5, §6. |

Parts not touched: `lucerna-editor`, `save.rs` (only calls the existing `confirm_quit`), the conflict bar, `merge`.

## 11. Error handling

- A corrupt `recent.json` is renamed to `.bak` and an empty list is used. A write error gives `tracing::warn` and is ignored, because the recent list must never block opening a folder.
- With no `HOME` (`default_data_dir()` is `None`) there is no recent list: Welcome only has Get Started, and every write is skipped.
- `prompt_for_paths` failing gives `tracing::error` and stays on the current screen.
- Starting a new process fails ("new window"): report through `status_message`. The folder to switch to cannot be opened (W8): keep the current window and report through `status_message`.
- The save-confirmation dialog is Cancelled (W8, W10): do nothing.

## 12. Testing

- **Unit (`recent.rs`):**
  - `record` puts the entry first, drops duplicates, truncates at 100;
  - `remove`, `clear`;
  - `existing` drops missing folders and entries of the wrong kind (a file in place of a folder);
  - writing again drops missing entries;
  - a corrupt or different-version file becomes `.bak` and an empty list;
  - atomic write (no `.tmp` left).
- **GPUI test (`TestAppContext`):**
  - Welcome draws exactly the first 5 entries plus "More…", hides RECENT when the list is empty, drops missing entries;
  - × removes an entry; ⌘1 opens the first entry (checked through the Router's fake open callback);
  - the picker filters by name and parent path; `ctrl+r` selects the next entry; ⌘Enter emits `Open { new_window: true }`; Escape closes the picker;
  - the watermark shows labels per the keymap and hides when `workbench.tips.enabled` is `false`;
  - `startupEditor: "none"` picks the dialog, other values pick Welcome (split the choosing logic into a pure function to test it).
- **Unit (`build_menus`):**
  - hides entries with unregistered IDs, and merges extra separators;
  - macOS has the "Lucerna" menu first;
  - Linux/Windows have no app menu: About is in Help, Exit is at the end of File, no Services/Hide;
  - the Welcome window disables items that need a workbench;
  - Open Recent ▸ has 10 entries and More…, Clear;
  - displayed keys take the first single-key binding, drop chords, and change with the user's keymap.
- **GPUI test:** dispatching `RunCommand { id: "workbench.action.toggleSidebarVisibility" }` flips the sidebar state; a binding in the `LucernaMenu` context does not run when typing the key in the workbench (the command runs only once).
- **Manual (required before considering it done, per design-principles §1.1):** run `Lucerna.app` from the Dock and put it next to Zed with no project open to compare spacing, font size, hover and empty state. Then try ⌃R in the workbench: Enter replaces the window, ⌘Enter opens a new window, Close Folder returns to Welcome.
- **Manual, the menu part:** compare the menu bar with Zed and VS Code. Check:
  - Edit has Emoji & Symbols and Start Dictation;
  - Help has the command search box;
  - editing `keybindings.json` changes the keys in the menu;
  - ⌘C in the file name field of the Open dialog still works.
