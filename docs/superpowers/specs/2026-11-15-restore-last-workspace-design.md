# Reopen the most recent workspace at startup — short design

Date: 2026-11-15. Status: the user chose "do it like Zed".

> **Superseded** by the multi-window spec
> (`2026-11-15-multi-window-design.md`, M12 and §6.6): `window.restoreWindows`
> defaults to `"all"`, reopening every window in `windows.json`; `recent.json`
> is only a fallback when the window list is empty.

## Intent

Open a folder, quit Lucerna, reopen it from the Dock, and Lucerna shows
Welcome instead of the folder just used. The user wants it like Zed: a
startup with no path reopens the most recent workspace.

## References

- **Zed:** `restore_on_startup`, default `"last_workspace"`: reopens the
  most recently used workspace. `"last_session"` reopens every window,
  `"none"` opens an empty window.
- **VS Code:** `window.restoreWindows`, default `"all"`: reopens every
  window. `"one"` reopens the most recently used window, `"folders"`,
  `"preserve"`, `"none"`. Welcome (`workbench.startupEditor`) only shows
  when there is nothing to reopen.

## Design

- Behavior follows Zed (`last_workspace`), setting name follows VS Code
  (a project principle): `window.restoreWindows`, default `"one"`.
  - `"none"`: as before (Welcome or the folder prompt per
    `workbench.startupEditor`).
  - Any other value (`"one"`, `"all"`, `"folders"`, `"preserve"`): reopen
    **one** workspace, since each Lucerna window is a process. Reopening
    several windows comes later.
- "The most recent workspace" is the first entry in `recent.json` that
  still exists (a folder or a standalone file), opened through the
  normal open path (`Router::open_path`).
- Applies only when startup has no path (Dock, Finder opening the app) and
  no open-file request arrives during the initial wait. `luc <path>`
  and "Open With" still open the requested path.
- No entry still exists: show Welcome as before.

## Known limits

Close Folder then quit still reopens that folder next time (the recent
list does not record closing a folder). VS Code `"one"` would open an
empty window.

## Tests

`startup.rs`: `"none"` does not reopen; the default and other values
reopen; picks the newest entry that still exists, skipping deleted ones;
an empty list gives nothing.
