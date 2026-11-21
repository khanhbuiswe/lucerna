# Code font size zoom (⌘+ / ⌘− / ⌘0)

Date 2026-11-17. Status: implemented (branch `claude/font-zoom`); manual check §6 not done yet. The first item of tier 1 in `docs/roadmap.md`.

## 1. Purpose

People moving from VS Code press ⌘+ to read code bigger (large screen, presenting, eye strain).

**Current state** (corrected after approval; the first draft wrongly said there was no command): there already are
`editor.action.fontZoomIn` / `fontZoomOut` (⌘= / ⌘−, menu View → Zoom In / Zoom Out), but they only
change the active `CodeView` (`CodeView::zoom`, clamped 6–40). Other tabs, tabs opened later, diff tabs and the terminal
keep the old size. There is no ⌘0, no ⌘+, and no feedback on the new size.

**Goal:** ⌘= / ⌘+ / ⌘− / ⌘0 zoom in, zoom out and reset the code font size in every view and every window.

**Out of scope:** zooming the whole UI (`window.zoomLevel`); ⌘ + mouse wheel (`editor.mouseWheelZoom`);
saving the zoom level across app launches.

**Anti-goal:** zooming changes the user's `settings.json`.

## 2. References

- **Zed** (`assets/keymaps/default-macos.json`): `cmd-=` / `cmd-+` → `zed::IncreaseBufferFontSize`,
  `cmd--` → `DecreaseBufferFontSize`, `cmd-0` → `ResetBufferFontSize`, all `{ "persist": false }`:
  nothing is written to settings, lost on restart. In Welcome and Onboarding, these keys change the UI font size.
  Zed's terminal uses `buffer_font_size` when it has no size of its own.
- **VS Code:** ⌘= / ⌘− zoom the whole window (`workbench.action.zoomIn`, writes `window.zoomLevel`). The
  editor alone has `editor.action.fontZoomIn` / `fontZoomOut` / `fontZoomReset`, with no default keys,
  not saved across launches.
- **Helix:** runs in a terminal, has no zoom.

## 3. Approaches considered

| Approach | Trade-off | Result |
|---|---|---|
| A. Zoom the whole UI like VS Code | Matches the "behavior follows VS Code" principle, but ~634 fixed `px(...)` spots in `lucerna-workbench` would have to change to a shared factor | Dropped, later if someone asks |
| **B. Only the code font size, like Zed** | Small, only touches where `editor.fontSize` is read. Deviates from VS Code on the ⌘+ key (VS Code zooms the whole window) | **Chosen** |

A deliberate deviation from VS Code: the ⌘+ key calls the editor's font zoom command instead of zooming the window, because Lucerna has
no window zoom and someone pressing ⌘+ almost always wants to read code bigger.

## 4. Design

- **Commands** (VS Code IDs): `editor.action.fontZoomIn`, `editor.action.fontZoomOut`,
  `editor.action.fontZoomReset`. In the command palette as "Editor Font Zoom In",
  "Editor Font Zoom Out", "Editor Font Zoom Reset".
- **Default keys** (following Zed, changeable in `keybindings.json`): `cmd+=` and `shift+cmd+=` (⌘+
  on a US keyboard) → in, `cmd+-` → out, `cmd+0` → reset. No `when`, works everywhere in the
  window. The View menu adds "Reset Zoom".
- **State:** one integer offset (px) shared across the whole process, all windows. The real font size =
  `editor.fontSize` + the offset, clamped to 6–100 (the same limits as the onboarding font size step,
  `FONT_SIZE_MIN` / `FONT_SIZE_MAX`). Each press is ±1 px. At the limit, do nothing.
- **Not saved:** the offset lives only in memory; nothing is written to `settings.json`, nothing goes into app state.
- **Settings win:** when `editor.fontSize` in the User layer changes (the user edits `settings.json`), the
  offset goes back to 0. A change in the Workspace layer does not reset it (the plan writer's decision: rare, and the zoom still adds
  onto the new size).
- **Applies to:** everywhere that draws code by `editor.fontSize` (editor, diff tabs, excerpt lists,
  language server log), and the terminal when `terminal.integrated.fontSize` is `null`. Markdown preview
  is unchanged. The old 6–40 limit in these places becomes 6–100. UI font sizes (sidebar, tabs, status bar,
  Quick Open) are unchanged.
- **Feedback:** after each press, the status bar briefly shows "Font Size: N" through the existing
  `status_message` mechanism.
- **Resizing keeps position:** the line at the top of the viewport stays at the top of the viewport after the size changes (as when changing
  `editor.fontSize` today; if it does not hold that today, make it hold).
- **Vim:** not affected, all the keys carry ⌘.

## 5. Left to the implementer

- How to get the zoomed font size to the views: the plan picks a GPUI global `FontZoom` (module
  `font_zoom.rs`), `WindowSettings` observes that global and then `notify`s, so every view already observing
  settings re-reads on its own.
- When the terminal already has its own size (`terminal.integrated.fontSize` is not `null`) the terminal does not zoom.
  This is the spec writer's decision (like Zed: the terminal follows the buffer only when it has no size of its own).

## 6. Testing

- Unit tests: the ±1 step, clamping to 6–100, reset, the offset going to 0 when settings change.
- Manual in the app: ⌘+ in the editor, a diff tab, the terminal; two windows change together; after a restart it returns to the size
  in settings; `settings.json` does not change.
