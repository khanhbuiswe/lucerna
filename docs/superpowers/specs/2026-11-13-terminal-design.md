# Integrated terminal — design

Date 2026-11-13. Part of phase 5 of the roadmap (§1.3 in `2026-11-08-lucerna-phase1-design.md`). Background research: `docs/research/2026-11-13-terminal.md`.

## 1. Purpose

Lucerna users let CLI agents (Claude Code, Codex) write the code. Today they have to run the agent in a separate terminal app and switch back and forth between two windows to both watch the agent work and read the files it edits. The integrated terminal brings the agent into the same window: the agent runs all session in a panel next to the code, and clicking a path the agent prints opens that file in the editor right away.

**Success criterion:** the author runs Claude Code all day in Lucerna's terminal panel instead of opening a separate terminal app. The agent renders correctly, typing has no lag, and reading code next to it while the agent keeps printing shows no jank.

### 1.1 Goals (MVP)

- Terminal panel at the bottom by default, movable to the right, toggled with ⌃`.
- Open the terminal in the workspace root, running the shell taken from `$SHELL`.
- Multiple terminals, each one a tab in the panel.
- Colors follow the theme.
- Select text, copy, paste, scroll back through what has gone by.
- Change the number of rows and columns when the panel is resized.
- ⌘+click on a URL or file path (with `:line:col`) opens it in the browser or in the editor.
- Smooth running of heavy TUIs such as Claude Code, Codex, vim, htop.
- Vietnamese typing through an input method (IME).

### 1.2 Outside the MVP

- Shell integration (OSC 633, command marks, scroll by command).
- Split terminal panes.
- Terminal as a tab in the editor area, dragging a terminal between the panel and the editor area.
- Search in the terminal (⌘F).
- Keeping terminals across app restarts (persistent sessions).
- Profile picker, changing the tab icon or color, renaming the tab.
- Left panel, floating panel.
- Problems, Output and other views in the panel. The panel is built so more can be attached later (§4.1), but the MVP has only Terminal.

### 1.3 Must not do (anti-goals)

- Do not slow down the editor. An agent printing continuously in the terminal must not push the editor's frame time past 16 ms.
- Do not use CPU when idle. A terminal with no new data does not redraw. The cursor does not blink by default.
- No AI features. The terminal is only a terminal.
- Do not copy code from Zed's `terminal` or `terminal_view` crates (GPL).

## 2. Constraints

- Only `lucerna-workbench` may use gpui. The terminal core lives in a new crate that does not use gpui.
- License MIT OR Apache-2.0; `cargo deny check` must pass. `alacritty_terminal` 0.26.0 has been checked (research §4).
- macOS first. Linux and Windows must work without a rewrite: do not assume macOS except where explicitly marked as platform-specific.
- The UI thread never waits on I/O. PTY reading and parsing run on their own thread.
- Behavior, commands, settings and keys follow VS Code; UI and performance follow Zed. Where they differ, the reason is stated.

## 3. Approaches considered

| Approach | Gains | Loses |
|---|---|---|
| **1. `alacritty_terminal` + a self-written GPUI renderer** (chosen) | Emulator that Zed already uses for real with Claude Code and vim; has ConPTY for Windows; Apache-2.0; binary grows ~290 KB; 7 new crates | Must write the cell-grid rendering and input ourselves |
| 2. `wezterm-term` + `portable-pty` | Good emulator, MIT | Not on crates.io, a git source must be added to `deny.toml`; heavy dependency tree; no GPUI editor uses it yet to learn from |
| 3. `vt100` + `portable-pty` | Light | No reflow on resize, mouse modes and alt screen uncertain; risk of drawing TUIs wrong |

The user chose approach 1.

**Panel position**, chosen by the user: bottom by default, movable to the right. Reason: the agent runs all session; a bottom panel on a laptop is only ~15–20 rows, while with the panel on the right the agent gets one column as tall as the screen and the code is still visible. Two options were rejected: bottom only with a maximize button (maximizing covers the code); terminal as an editor tab (left outside the MVP, it drags in editor group changes).

## 4. UI

Follows Zed (`TerminalPanel` at revision `b5cd310`, research §2.4).

### 4.1 Panel

```
Bottom position (default):
┌ title bar ───────────────────────────────────────────────┐
├ sidebar ┬ editor area ───────────────────────────────────┤
│         │                                                 │
│         ├═════════════ sash 6 px ═════════════════════════┤
│         │ ▣ zsh  ▣ claude ×                     ＋  ⤢  × │ ← tab bar
│         │ $ claude                                        │
│         │ ╭─────────────────────────────╮                 │
│         │ │ ✻ Welcome to Claude Code    │                 │ ← terminal body
│         │ ╰─────────────────────────────╯                 │
├ status bar: [📁][🔍][⊞][▣] …                              ┤
└──────────────────────────────────────────────────────────┘

Right position:
┌ sidebar ┬ editor area ─────────┬║┬ ▣ claude ×   ＋ ⤢ × ┐
│         │                      │║│ $ claude            │
│         │                      │║│ …                   │
└─────────┴──────────────────────┴║┴─────────────────────┘
```

- A bottom panel takes the width of the **editor area**, not spilling over the sidebar (like Zed with the bottom dock). A right panel takes the full height between the title bar and the status bar.
- Default size: 320 px high at the bottom, 640 px wide on the right (as in Zed). Drag the sash to change; minimum 4 rows or 20 columns, maximum leaves the editor area at least 120 px. The size for each position, the current position and the shown/hidden state are saved in `WorkspaceState` like `sidebar_width`.
- Panel background `panel.background`; 1 px `border` on the edge next to the editor area. The sash reuses the sidebar's sash approach (`layout/sidebar.rs`), split into a function shared in `ui.rs` for both orientations.
- The panel is written as a shared frame (`layout/panel.rs`): position, size, sash, show/hide, maximize. Terminal is the only content in the MVP. No general "view container" system like VS Code's is built.
- **Maximize** (`workbench.action.toggleMaximizedPanel`, the ⤢ button): the panel takes the whole editor area; pressing again restores the old size.

### 4.2 Tab bar in the panel

- The tab bar is **always shown**, even with a single terminal (like Zed; VS Code only shows the tab list on the right when there are two or more terminals). Height, background `tab_bar.background`, active/inactive tab colors and bottom border are **identical to the editor area's tab bar**. Split the tab drawing now in `editor_groups/view.rs` into a shared component in `ui.rs`, then use it in both places.
- Each tab: the `TERMINAL` icon (already in `icons.rs`), the name, an × button on hover or when active. The tab name follows §6.6 (for example `zsh`, `claude`, `vim`), truncated with `…` when longer than 24 characters.
- A tab with a bell or a process that exited with a non-zero code changes the name color (`warning` / `error`).
- On the right of the tab bar: `＋` (new terminal), `⤢` (maximize/restore), `×` (hide the panel; does not kill terminals). Use the existing `bar_icon_button`. Different from Zed: Zed only shows the buttons when the pane has focus; Lucerna always shows them, because there are only three buttons and no split.
- Middle-click on a tab closes the terminal. Drag a tab to reorder.

### 4.3 Terminal body

- Background `terminal.background`. Left margin of one cell, like Zed.
- The vertical scrollbar uses `scrollbar_thumb`, shown only while scrolling or on hover, like the editor.
- When focus is lost, the block cursor becomes an empty outline (`cursorStyleInactive: outline`).

### 4.4 Status bar button

Add a terminal button to the `panel_toggles` group at the left end of the status bar, after Extensions. Icon `TERMINAL`, tooltip "Terminal (⌃`)", lit while the panel is shown. Clicking runs `workbench.action.terminal.toggleTerminal`.

## 5. Architecture

### 5.1 Crate `lucerna-terminal` (new, no gpui)

Depends on `alacritty_terminal = "0.26"`. All of alacritty's types are hidden behind this crate; the workbench only sees Lucerna's types. It contains:

- **`Terminal`**: one terminal session. Holds `Arc<FairMutex<Term<_>>>`, a `Notifier` for writing to the PTY, and a queue of operations from the UI. Public API (names are only suggestions):
  - `spawn(TerminalOptions) -> Result<Terminal>`: `TerminalOptions` includes shell, args, cwd, env, rows/columns, cell size, scrollback line count.
  - `input(&[u8])`, `paste(&str)` (wraps in bracketed paste automatically when the app enabled that mode, filters ESC characters in the pasted content).
  - `resize(rows, cols, cell_w, cell_h)`: writes only when different from last time; consecutive resizes are coalesced.
  - `scroll(Scroll)`, `select_start/update/end(point, kind)`, `selection_text()`, `clear()` (clears scrollback and screen, keeps the prompt line), `select_all()`.
  - `snapshot() -> Snapshot`: takes the lock **once**, applies the operation queue, copies the **visible** cells along with the cursor, selection, mode (alt screen, mouse mode, bracketed paste, app cursor), scroll position, total line count.
  - `mouse_report(event) -> Option<Vec<u8>>`, `key_to_bytes(key, mods, mode) -> Option<Vec<u8>>`: encode keys and mouse by the current mode. Pure functions, testable.
  - `foreground_process() -> Option<ProcessInfo { name, cwd }>`: see §6.6.
- **Event thread.** alacritty's `EventLoop` runs on its own thread, reading and parsing the PTY (each lock hold parses at most 64 KB, built into alacritty). The listener sends events into a channel. The receiving side batches in a **4 ms** window: the first event is handled immediately, `Wakeup`s are merged into one, at most 100 events per batch. Raw bytes do not go through the channel. The result handed to the workbench is `TerminalEvent`: `Wakeup`, `Title(String)`, `Bell`, `Exited(Option<i32>)`, `ClipboardStore(String)`.
  - `PtyWrite` (replies to app queries, for example reporting the cursor position) is written straight to the PTY in order.
  - `ClipboardLoad` (OSC 52 read of the clipboard) is **rejected**: apps in the terminal must not read the clipboard. `ClipboardStore` (OSC 52 write) is allowed, because Claude Code and vim use it to copy over SSH.
- **Run merging** (`runs.rs`): a pure function that takes one row of snapshot cells and returns a list of `TextRun { col_start, text, cells, style }` and `BgRect { col_start, cols, color }`. Adjacent cells with the same font, text color, background color, underline, strikethrough merge into one run; adjacent backgrounds of the same color merge into one rectangle. Empty cells without a background are dropped, the padding cell of a wide character is dropped, zero-width characters are attached to the previous cell. Block characters U+2580–U+259F are returned as rectangles instead of text.
- **Color** (`color.rs`): converts cell colors (ANSI names, indexes 0–255, truecolor) to RGBA by `TerminalPalette` (16 theme colors + foreground/background/cursor/selection). Indexes 16–231 are the xterm color cube, 232–255 the gray ramp. Has a minimum contrast function (WCAG, §7).
- **Link detection** (`links.rs`): a pure function on one logical line (wrapped lines joined). Order: OSC 8, URL (`http`, `https`, `file`, `mailto`), then a path with or without a position suffix. Accepted suffixes: `:L`, `:L:C`, `(L,C)`, `(L)`, `, line L`, `", line L"` (Python traceback). Has a time budget of 1 ms per detection.

### 5.2 In `lucerna-workbench`

New module `terminal/`:

- `TerminalPanel` (Entity): the list of `TerminalTab`, the active tab, calls the panel frame of §4.1.
- `TerminalView` (Entity, Focusable): one terminal. Holds `lucerna_terminal::Terminal`, the last snapshot, mouse state, the link being hovered. Receives `TerminalEvent` through a background task; on `Wakeup` it calls `cx.notify()`, so it redraws only when there is new data.
- `TerminalElement` (Element): prepaint measures the cell, calls `resize` if rows/columns changed, takes `snapshot()`, calls run merging for the visible rows; paint draws in this order: background, selection, text, block characters, underline of the hovered link, cursor, IME composing text.
- Commands registered in `CommandRegistry` (§8).
- `lucerna-theme`: add `TerminalColors` to `Theme`, reading Zed theme keys (§7).

### 5.3 Why this way

Splitting out a crate lets us test the core (key → bytes, run merging, link detection, color) without GPUI, and keeps the rule that only the workbench uses gpui. The adapter layer, learned from Zed, means that later swapping the emulator or moving to a new alacritty version only touches one crate.

## 6. Behavior

### 6.1 Open and close

- ⌃` (`toggleTerminal`): if the panel is hidden, show it and focus the active terminal; create one if there is none. If the panel is shown but focus is elsewhere, focus the terminal. If focus is in the terminal, hide the panel and return focus to the editor. Like VS Code.
- ⌃⇧` (`new`): create a new terminal, show the panel, focus it.
- Hiding the panel does not kill terminals; processes keep running.
- **Process exit** (following VS Code): code 0 closes the tab. A non-zero code or being killed by a signal keeps the tab, prints an extra line `[Process exited with code N]` (color `text_muted`), and the tab name changes to the `error` color. Pressing any key in that tab closes it.
- Closing the last tab hides the panel (`terminal.integrated.hideOnLastClosed`, default true).
- `kill` (and × on the tab) sends SIGHUP to the process group then closes the tab, without asking (VS Code's `confirmOnKill` by default only asks for terminals in the editor area).
- Closing the window or quitting the app while terminals are still running: **no prompt** (`terminal.integrated.confirmOnExit` default `never`, like VS Code). The values `always` and `hasChildProcesses` are supported.
- Terminals are not kept across app restarts. The panel state (shown/hidden, position, size) is kept. If the panel was shown when closing, the next launch shows the panel with a new terminal.

### 6.2 Shell, directory, environment

- **Shell:** `terminal.integrated.defaultProfile.<os>` points to an entry in `terminal.integrated.profiles.<os>` (`path`, `args`, `env`). If unset, use `$SHELL`; with no `$SHELL`, `/bin/zsh` on macOS, `/bin/sh` on Linux, `powershell.exe` on Windows.
- **Login shell on macOS:** when using the default shell, add `-l` (zsh accepts `-l`; VS Code uses `-l`). An app launched from Finder has a minimal `PATH`; the login shell loads the user's `PATH` (Homebrew, nvm, rustup), which lets `claude` run. A profile that sets its own `args` uses exactly those `args`. Linux and Windows do not add it.
- **Directory:** `terminal.integrated.cwd` if set (absolute, or relative to the workspace root); otherwise the workspace root. In single-file mode, the folder containing that file. A directory that does not exist falls back to the home directory and prints a warning line.
- **Environment:** the Lucerna process's env, then `terminal.integrated.env.<os>`, then the profile's `env`; a `null` value removes the variable. Always set:
  - `TERM=xterm-256color`, `COLORTERM=truecolor`
  - `TERM_PROGRAM=lucerna`, `TERM_PROGRAM_VERSION=<version>`
  - `LANG=en_US.UTF-8` if `LANG` is missing or not UTF-8 (VS Code's `detectLocale: auto`; GUI apps on macOS often lack `LANG`, which breaks Vietnamese).
- No `inheritEnv` in the MVP (always inherits).

### 6.3 Restricted Mode

**Decision: the terminal also runs in untrusted folders.** The user chose this.

- VS Code blocks: opening a terminal in Restricted Mode shows a trust dialog, and pressing Cancel does not open it. There is `terminal.integrated.allowInUntrustedWorkspace` (default false) to skip the check. Zed does not block a manually opened terminal.
- Lucerna follows Zed, **deviating from VS Code**. Reason: users open the terminal to run an agent, and the agent runs code in the repo by itself. Asking for trust when opening a terminal protects nothing extra, it only adds a click to the most frequent task of the day.
- The safety parts of both sides are still kept: when the folder is untrusted, the settings `terminal.integrated.cwd`, `env.*`, `profiles.*`, `defaultProfile.*` **at the Workspace layer are ignored**, because a foreign repo could use them to point the shell at a malicious executable. Read through the settings store's existing function that skips the Workspace layer (`store.rs`). Values at the User layer still apply.
- `allowInUntrustedWorkspace` is not added, because there is nothing to allow.
- Opening a terminal does not raise a trust dialog, and does not count as the language server's "first time needing trust".

### 6.4 Keyboard

When focus is in the terminal, each key goes through the workbench's `dispatch_key` as usual:

1. Vim mode does **not** receive keys when focus is in the terminal.
2. The keymap is resolved with context `terminalFocus` (plus `terminalTextSelected`, `terminalAltBufferActive` when true). If it yields a command **and** that command is in the "skip shell" list, the workbench runs that command.
3. Otherwise, the key is encoded by the terminal's mode (app cursor, app keypad, function keys, xterm-style modifiers `CSI 1;m`) and sent to the PTY.

Skip-shell list = the default list in §8.1, plus `terminal.integrated.commandsToSkipShell` (add IDs, or remove with the `-` prefix), like VS Code. ⌃ and ⌥ keys go down to the shell unless they match a command in the list (⌃`, ⌃⇧`). Chords: `allowChords` default true, like VS Code.

**Keys with ⌘ on macOS specifically:** the shell cannot receive ⌘, so a key with ⌘ **never** goes down to the PTY, even when the command is not in the skip-shell list. If it matches a command (by `when`) the command runs; otherwise it is dropped. The `-` prefix in `commandsToSkipShell` only affects keys without ⌘. Consequences, following VS Code:
- ⌘W runs `workbench.action.closeActiveEditor` (the keybinding has no `when`): it closes the active editor tab, and does **not** close the terminal. Like VS Code with the terminal in the panel. Accidentally killing the agent is worse than accidentally closing a file that can be reopened.
- ⌘S, ⌘\, ⌘P… run as when focus is in the editor.
- ⌘= / ⌘- (`editor.action.fontZoomIn/Out`) only change the editor text, not the terminal text, like VS Code. Separate zoom for the terminal comes later.

**Focus:** the panel has its own focus handle, **outside** the `editor_focus` tree, so `editorFocus` is false when focus is in the terminal. That way vim (`editorFocus && !textInputFocus`) and editor-only commands (⌘C → `clipboardCopyAction`) do not capture the terminal's keys.

- **Option:** `terminal.integrated.macOptionIsMeta` default false (like VS Code). When false, ⌥ produces characters per the macOS keyboard; when true, ⌥+key sends `ESC` + the key.
- **Zed's line-editing shortcuts** (macOS only): ⌘← sends `^A`, ⌘→ sends `^E`, ⌘⌫ sends `^U`, ⌥← sends `ESC b`, ⌥→ sends `ESC f`. VS Code does not have these by default but macOS users are used to them from Terminal.app/iTerm. Written as `sendSequence` keybindings in `default-keybindings.json` with `when: terminalFocus && !terminalAltBufferActive`, so users can remove them. In the alt screen (vim, htop, Claude Code in full-screen mode) they do not run: ⌘ is dropped, ⌥ is encoded normally down to the app.
- **IME:** `TerminalView` installs GPUI's `InputHandler`. The composing text is drawn at the cursor, underlined; on commit the UTF-8 string is sent to the PTY. The cursor cell position is reported to the IME so the candidate window appears in the right place. Required for Vietnamese input methods.

### 6.5 Mouse, selection, copy, paste, scroll

- **Select:** drag to select (2 px threshold), double-click selects a word (by `terminal.integrated.wordSeparators`), triple-click selects a logical line, ⇧+click extends the selection. ⌥+drag selects a rectangle.
- When the app enables mouse mode (vim, htop, Claude Code), click and drag are reported to the app. Holding ⇧ selects text as normal (like Zed); `macOptionClickForcesSelection` makes ⌥ also force selection (like VS Code, default false).
- **Copy:** ⌘C when there is a selection (`terminalTextSelected`); with no selection ⌘C does nothing (⌃C is still interrupt). The selection is kept after copying. `copyOnSelection` default false. Linux: also writes the primary selection.
- **Paste:** ⌘V; multi-line paste gives no warning if the app enabled bracketed paste, and warns once if not (`enableMultiLinePasteWarning: auto`).
- **Right click:** `rightClickBehavior` default on macOS is `selectWord`. The MVP supports `selectWord`, `copyPaste`, `paste`, `nothing`, `default`. There is no context menu yet, so `default` behaves like `selectWord`.
- **Scroll:** the mouse wheel scrolls the scrollback by `mouseWheelScrollSensitivity`, ⌥ scrolls fast by `fastScrollSensitivity`. In the alt screen: if the app enabled mouse mode, scrolling is reported to the app; if not, sends ↑/↓ keys (alternate scroll). Typing a key jumps to the bottom. If new data arrives while scrolled up, the viewed position stays put.
- `scrollUp/Down`, `scrollUpPage/DownPage`, `scrollToTop/Bottom` only work on the main screen, like VS Code.

### 6.6 Tab name and current directory

- On each `Wakeup` (at most once per 500 ms, on a background thread), read the PTY's foreground process: `tcgetpgrp` on the PTY's fd, then that process's name and cwd (macOS: `proc_pidinfo`; Linux: `/proc/<pid>/{comm,cwd}`; Windows: later). The `sysinfo` crate is not used for just this. The function lives in `lucerna-terminal`, with a platform-specific part.
- **Tab name**, in order:
  1. The OSC 0/2 title if the **current** foreground process has set one (Claude Code sets titles like `✳ <current task>`). When the foreground process changes, the old title is dropped, so the tab does not keep the name of a command that has exited. If the shell sets the title itself (oh-my-zsh…), use it as is.
  2. The foreground process name. If it is an interpreter (`node`, `bun`, `deno`, `python`, `python3`, `ruby`), take the file name of the first script argument in argv (Claude Code and Codex installed through npm run under `node`, so the tab shows `claude` instead of `node`). Reading argv: macOS `sysctl KERN_PROCARGS2`; Linux `/proc/<pid>/cmdline`.
  3. `Terminal`.
  - The tab tooltip shows the process name and cwd.
- The current cwd is used to resolve relative links (§6.7).

### 6.7 Links

Following VS Code for how to open, following Zed for how to detect.

- Hold ⌘ (Linux, Windows: ⌃) and hover: the link under the mouse is underlined, the mouse cursor becomes a hand, the tooltip shows the destination and "⌘+click to open". Detection only runs while ⌘ is held, at most once per 100 ms or per 5 px.
- ⌘+click:
  - **URL:** open in the default browser if the scheme is in `terminal.integrated.allowedLinkSchemes` (default `http`, `https`, `mailto`, `file`). Other schemes are not opened. VS Code asks then adds the scheme to the setting; the MVP drops that dialog.
  - **Path:** resolve in order, stopping at the first that exists (`stat` check on a background thread):
    1. absolute, or `~/…`;
    2. relative to the terminal's current cwd (§6.6);
    3. relative to the cwd when the terminal was opened;
    4. relative to the workspace root;
    5. strip the `a/` or `b/` prefix (`git diff` output) then retry 2–4.
  - File: open in the editor via `open_path_at`, placing the cursor at the line/column if present. A file outside the workspace still opens.
  - Directory in the workspace: reveal in the Explorer. Directory outside the workspace: do nothing.
  - Not found: no link (no underline). The MVP has no VS Code "search in workspace" fallback.
- OSC 8 also needs ⌘+click like every other link (VS Code), not opening on a plain click (different from Zed), so that clicks inside Claude Code do not accidentally open a browser.
- `terminal.integrated.enableFileLinks`: `on` (default) or `off`.

### 6.8 Bell

`terminal.integrated.enableBell` default false. When on, the tab changes to the `warning` color until a key is pressed in that tab. No sound is played.

## 7. Color and font

### 7.1 Theme keys

Lucerna's themes come from Zed extensions, so it reads the **Zed theme keys**:

| Zed key | Used for | When missing |
|---|---|---|
| `terminal.background` | background | `editor.background` |
| `terminal.foreground` | default text | `editor.foreground` |
| `terminal.bright_foreground`, `terminal.dim_foreground` | default bold/dim text | `terminal.foreground` |
| `terminal.ansi.{black,red,green,yellow,blue,magenta,cyan,white}` | colors 0–7 | VS Code's default table (dark/light by `appearance`), research §1.5 |
| `terminal.ansi.bright_*` | colors 8–15 | same as above |
| `terminal.ansi.dim_*` | dim colored text | the normal color with alpha 0.7 |
| `players[0].cursor` | cursor | the UI's `cursor` |
| `players[0].selection` | selection | the UI's `selection` |

Drawn colors come from the selected theme and change immediately when the theme changes.

### 7.2 Minimum contrast

`terminal.integrated.minimumContrastRatio` default 4.5 (WCAG, like VS Code; 1 turns it off). Applies only to colors coming from the theme (the 16 ANSI colors and the default colors), **not** to indexes 16–255 and truecolor (the app chose them exactly), and not to block characters. Like Zed. Results are cached per (fg, bg) pair.

### 7.3 Font

| Setting | Default | Notes |
|---|---|---|
| `terminal.integrated.fontFamily` | empty, i.e. `editor.fontFamily` | |
| `terminal.integrated.fontSize` | empty, i.e. `editor.fontSize` | Deviates from VS Code (12 on macOS): so the terminal and editor match like Zed |
| `terminal.integrated.lineHeight` | 1.2 | A multiplier. VS Code 1, Zed 1.3; 1 makes Claude Code's box-drawing frames touch tightly, 1.3 wastes rows on a bottom panel |
| `terminal.integrated.letterSpacing` | 0 | |
| `terminal.integrated.fontWeight` / `fontWeightBold` | `normal` / `bold` | |
| `terminal.integrated.drawBoldTextInBrightColors` | true | |
| `terminal.integrated.cursorStyle` | `block` | `line`, `underline` |
| `terminal.integrated.cursorBlinking` | false | |

Ligatures off. The cell width is the advance of `m`. The terminal height is rounded to a whole number of rows; the remainder goes on top when stuck to the bottom, at the bottom when not.

## 8. Commands, keys, settings

### 8.1 Default commands and keys (macOS)

Every command below is in the default skip-shell list.

| Command | Key | when |
|---|---|---|
| `workbench.action.terminal.toggleTerminal` | ⌃` | |
| `workbench.action.terminal.new` | ⌃⇧` | |
| `workbench.action.terminal.focus` | | |
| `workbench.action.terminal.kill` | | |
| `workbench.action.terminal.focusNext` / `focusPrevious` | ⌘⇧] / ⌘⇧[ | `terminalFocus` |
| `workbench.action.terminal.clear` | ⌘K | `terminalFocus` |
| `workbench.action.terminal.copySelection` | ⌘C | `terminalFocus && terminalTextSelected` |
| `workbench.action.terminal.paste` | ⌘V | `terminalFocus` |
| `workbench.action.terminal.selectAll` | ⌘A | `terminalFocus` |
| `workbench.action.terminal.clearSelection` | Esc | `terminalFocus && terminalTextSelected` |
| `workbench.action.terminal.scrollUp` / `scrollDown` | ⌘⌥PageUp / ⌘⌥PageDown | `terminalFocus && !terminalAltBufferActive` |
| `workbench.action.terminal.scrollUpPage` / `scrollDownPage` | PageUp / PageDown | same as above |
| `workbench.action.terminal.scrollToTop` / `scrollToBottom` | ⌘Home / ⌘End | same as above |
| `workbench.action.terminal.sendSequence` | (args `{ "text": "…" }`) | |
| `workbench.action.togglePanel` | ⌘J | |
| `workbench.action.toggleMaximizedPanel` | | |
| `workbench.action.positionPanelBottom` / `positionPanelRight` | | |

In addition there are the ⌘←, ⌘→, ⌘⌫, ⌥←, ⌥→ keys in `sendSequence` form in §6.4.

**Linux and Windows:** add ⌃⇧C / ⌃⇧V for copy/paste like VS Code; ⌃` stays the same.

**Note for Claude Code:** Claude Code's `/terminal-setup` command adds to VS Code's keybindings an entry ⇧Enter → `sendSequence`. With `sendSequence`, Lucerna users can do the same in `keybindings.json`.

The default skip-shell list also includes non-terminal workbench commands: `workbench.action.quickOpen`, `workbench.action.showCommands`, `workbench.action.findInFiles`, `workbench.action.toggleSidebarVisibility`, `workbench.view.explorer`, `workbench.view.extensions`, `workbench.action.focusFirstEditorGroup`…`Eighth`, `workbench.action.quit`, `workbench.action.gotoLine` (VS Code also has equivalent commands in `DEFAULT_COMMANDS_TO_SKIP_SHELL`).

### 8.2 New settings

Besides the font settings (§7.3): `defaultProfile.{osx,linux,windows}`, `profiles.{osx,linux,windows}`, `cwd`, `env.{osx,linux,windows}`, `scrollback`, `copyOnSelection`, `rightClickBehavior`, `macOptionIsMeta`, `macOptionClickForcesSelection`, `minimumContrastRatio`, `confirmOnExit`, `hideOnLastClosed`, `enableBell`, `enableFileLinks`, `allowedLinkSchemes`, `wordSeparators`, `commandsToSkipShell`, `allowChords`, `mouseWheelScrollSensitivity`, `fastScrollSensitivity`, `enableMultiLinePasteWarning`, `cursorStyleInactive`, `detectLocale` — all under the prefix `terminal.integrated.`, with default values like VS Code except where noted otherwise here. Add `workbench.panel.defaultLocation` (`bottom` | `right`, default `bottom`).

Changing font, color, scrollback settings applies immediately to open terminals. Changing shell, cwd, env only applies to terminals opened afterwards.

## 9. Performance

### 9.1 Drawing

- **Per row, merged runs:** each visible row yields text runs and background rectangles as in §5.1. Each run is shaped once with `shape_line` using a fixed cell width so glyphs fit the grid.
- **Redraw only on new data:** `TerminalView` only calls `notify` on `Wakeup` (already merged over 4 ms), on user actions (select, scroll, link hover), on focus change, or on resize. An idle terminal produces no frames, CPU ~0.
- **Never walk the scrollback while drawing.** The snapshot only contains the visible part.
- Hidden panel: no drawing, but the PTY thread still reads and parses so the process is not blocked.
- **Next step if measurement shows it is slow:** alacritty has per-row damage tracking. If the prepaint of a 200×60 terminal exceeds 2 ms, cache the shape results per row and only reshape damaged rows. Not done in the MVP if the numbers pass.

### 9.2 Thresholds

| Measure | Threshold |
|---|---|
| From key press to the character appearing (shell echo) | ≤ 1 frame after the PTY returns bytes; total ≤ 16 ms at 60 Hz on M1 |
| Prepaint + paint of a 200-column × 60-row terminal full of colored text | ≤ 4 ms on M1, release build |
| `cat` of a 100 MB file into the terminal | UI does not freeze; the editor next to it still scrolls, frames ≤ 16 ms; terminal finishes in ≤ 3 s |
| Idle terminal | 0 frames per second |
| Scrollback RAM | `scrollback` default **10,000 lines** (VS Code 1000, Zed 10,000). Estimated max ~30 MB per terminal at 120 columns (an alacritty cell is ~24 bytes; rows are only allocated when used). Hard cap 100,000 lines: a larger setting is clamped |
| Whole-app RAM with 1 terminal running Claude Code | grows ≤ 50 MB compared with no terminal |
| Release binary | grows ≤ 1.5 MB (measured: alacritty + PTY ~290 KB, the rest for the workbench) |

**Deviation from VS Code on scrollback:** VS Code defaults to 1000 lines. The output of an agent session is much longer, and 1000 lines loses the part the user needs to scroll back to. Lucerna takes Zed's default of 10,000.

### 9.3 Threads

- PTY thread: read, parse, hold the lock at most 64 KB at a time (alacritty). Never touches GPUI.
- UI thread: takes the lock once per frame in `snapshot()`, held briefly. No file reads, no `stat` (link resolution runs in the background).
- Foreground process reading: background thread, rate limited.

## 10. Platforms

- **macOS:** the main target. Login shell §6.2.
- **Linux:** runs: alacritty's PTY, `/proc` for the foreground process, primary selection, ⌃⇧C/⌃⇧V.
- **Windows:** ConPTY through alacritty; default shell `powershell.exe`; foreground process name comes later (the tab name uses the shell name). Not tested on Windows in the MVP, but code must not assume Unix except in explicit `#[cfg(unix)]` places.
- `deny.toml`: when starting builds for Linux/Windows, add targets to `[graph] targets`. This spec does not change that file.

## 11. Testing

- **`lucerna-terminal` (unit, no GPUI needed):** key encoding by mode (app cursor, modifiers, function keys, whether ⌥ is Meta); mouse encoding (SGR, normal); run merging (same style merges, different style splits, wide characters, zero-width, block characters, adjacent backgrounds); color (256 table, truecolor, contrast, not applied to 16–255); link detection (URL, `path:L:C`, `(L,C)`, Python traceback, git prefix, a link wrapped across two lines, time budget); snapshot after feeding a byte string (cursor, alt screen, selection); env and args on spawn (login shell, `LANG`, `TERM_PROGRAM`, dropping the Workspace layer when untrusted).
- **Real spawn** (integration test, Unix): open `/bin/sh -c 'printf hi'`, wait for `Exited(Some(0))`, the snapshot has `hi`. Resize changes `stty size`.
- **`lucerna-workbench` (gpui test):** keys with ⌘ never reach the PTY, ⌘W with focus in the terminal closes the editor not the terminal; ⌃` toggles and moves focus correctly in the three cases; new tab, close tab, closing the last tab hides the panel; keys in the skip-shell list run the command, other keys go to the PTY; vim does not receive keys when focus is in the terminal; changing the panel position; the size is saved; a process exiting with code 0 closes the tab, a non-zero code keeps the tab.
- **Manual before calling it done** (side by side with Zed, design-principles §1.1): Claude Code (frame drawing, colors, scrolling while running, ⇧Enter via `sendSequence`), Codex, vim (mouse mode, alt screen), htop, `cat` of a large file, resizing the panel while Claude Code is drawing, typing Vietnamese with the Telex input method in the shell and in Claude Code, ⌘+click on a path Claude Code prints, changing theme while the terminal is open. Compare spacing, tab bar height, background colors, borders against Zed.
- Measure the §9.2 thresholds and record the numbers in the plan's ledger.

## 12. Order of work (suggestion for the plan)

1. Crate `lucerna-terminal`: spawn, event thread, snapshot, key/mouse encoding, run merging, color. Unit tests.
2. `lucerna-theme`: `TerminalColors`.
3. Panel frame (`layout/panel.rs`), shared sash, split out the tab component, status bar button, state saving.
4. `TerminalView` + `TerminalElement`: drawing, resize, focus, keys, IME.
5. Select, copy, paste, scroll, mouse.
6. Links.
7. Settings, keybindings, Restricted Mode, tab name.
8. Measure performance, review the UI next to Zed.

## 13. Left to the implementer

- Concrete type and function names in `lucerna-terminal`.
- How files are split in the `terminal/` module.
- Details of key encoding (follow xterm; alacritty has a ready table for a few cases, the rest is written from the xterm ctlseqs docs, not copied from Zed).
- The exact value of the path regex, as long as it passes the tests in §11.

## 14. References

- `docs/research/2026-11-13-terminal.md`: VS Code (commands, settings, links, trust, env) and Zed (architecture, drawing, UI, performance) at revision `b5cd310`, with the license check.
- Zed: `crates/terminal`, `crates/terminal_view` (GPL, learn the design only).
- VS Code: `src/vs/workbench/contrib/terminal/`, `terminalContrib/links/`, the docs `docs/terminal/*`, `docs/editor/workspace-trust`.
- xterm control sequences: <https://invisible-island.net/xterm/ctlseqs/ctlseqs.html>.
