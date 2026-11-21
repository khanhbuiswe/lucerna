# New File (⌘N) and Save As (⇧⌘S)

Date 2026-11-17. Status: implemented (branch `claude/untitled-save-as`); manual check §6 not done yet. The second item of tier 1 in `docs/roadmap.md`.

## 1. Purpose

People moving from VS Code are used to pressing ⌘N to open an empty tab, typing code, then ⌘S to name
it and save it into the project. They are also used to ⇧⌘S to save the open file under another name.
Lucerna has neither yet; to create a new file they must use Explorer → New File (naming it before typing).

The user confirmed that ⌘N is for **creating a new file that will be saved into the project**, not
mainly for scratch notes.

**Goals:**

- ⌘N opens an Untitled tab; ⌘S on that tab asks where to save and turns the tab into a real file, keeping undo and the cursor.
- ⇧⌘S saves any file to another path; the tab switches to the new file.
- No lost text: an unsaved Untitled goes through hot exit like a normal file.

**Out of scope:** automatic language detection (`workbench.editor.languageDetection`); the "Select a
language…" hint line on an empty tab (`workbench.editor.empty.hint`); `:w name` in Vim; Save As to a place that is not
a disk; opening Untitled from the CLI (`luc -n`); `workbench.editor.untitled.labelFormat` (always
`content`); an Untitled carrying a target path (`associatedResource` in VS Code); Save Copy As.

**Anti-goal:** an Untitled being written to disk as a file named `untitled:Untitled-1`, being handed to git, LSP,
the watcher; or losing text in an Untitled when the app quits without asking.

## 2. References

- **VS Code** (`src/vs/workbench/services/untitled/common/untitledTextEditorService.ts`): every
  document is identified by a URI. Untitled is `untitled:Untitled-N`, `N` the smallest number from 1 not yet used
  (`do { … counter++ } while (mapResourceToModel.has(...))`). The file service only reads/writes schemes it knows,
  so an Untitled cannot be accidentally written to disk. The default tab name takes the first line
  (`workbench.editor.untitled.labelFormat: "content"`). ⌘N = `workbench.action.files.newUntitledFile`,
  ⇧⌘S = `workbench.action.files.saveAs`. Saving an Untitled opens the Save As dialog; Save All also asks for each
  Untitled. Autosave skips Untitled. Hot exit backs up Untitled. `files.defaultLanguage` sets the language
  for a new Untitled.
- **Zed** (`crates/language/src/buffer.rs`): `Buffer.file: Option<Arc<dyn File>>`, Untitled is
  `None` ("`None` when there is no path"); a buffer is identified by its own id. Save As assigns the file through
  `file_updated`. The tab name also takes the first line. ⌘N = `workspace::NewFile`.
- **Helix:** `Document.path: Option<PathBuf>`, identified by `DocumentId`; `:new` creates a document with
  `path = None`, `:w <path>` assigns the path.

## 3. Approaches considered

| Approach | Trade-off | Result |
|---|---|---|
| **A. A fake path** `untitled:Untitled-N` (close to VS Code) | Small: `Documents`, tabs and hot exit are already keyed by path, so most of it works as is. Risk: any place that touches the disk and forgets to check would treat it as a real file. Guarded by a single check function and a test for each place | **Chosen** (user) |
| B. A separate type (path or Untitled N), like Zed / Helix | Safest, the compiler catches every place. But it requires refactoring `documents.rs` (~3000 lines), tabs, hot exit, every function that takes `&Path` | Dropped: many times the cost for the same behavior |
| C. No Untitled: ⌘N asks for a name right away and opens a real file | Nearly free, but deviates from VS Code and forces thinking of a name before typing | Dropped |

## 4. Design

### 4.1 Untitled identity

- An Untitled has the fake path `untitled:Untitled-N` (a relative `Arc<Path>`, never
  joined with the root directory). `N` is the smallest number from 1 not yet used **in that window** (a closed
  tab returns its number).
- A single function `is_untitled(path: &Path) -> bool` (the path starts with `untitled:`). Every place that
  touches the disk or an outside service checks it and skips Untitled:
  - saving (always goes the Save As path, §4.3), revert, autosave;
  - the watcher and handling of on-disk changes / conflicts;
  - git (gutter, Explorer colors, diff);
  - LSP (do not open, do not send `didOpen`);
  - Explorer: auto-reveal, marking modified files;
  - commands that take the path: Copy Path, Copy Relative Path, Reveal in Finder, Open Containing Folder —
    for Untitled they do nothing (the tab's right-click menu, if any, shows them dimmed);
  - Markdown preview, workspace state (Untitled is not written to the list of tabs to reopen; hot exit
    handles that, §4.5), the Recent list.
- A real file whose name starts with `untitled:` (valid on macOS) always has an absolute path, so there is no
  collision: `is_untitled` is only true for a relative single-component path.

### 4.2 ⌘N — New File

- Command `workbench.action.files.newUntitledFile`, titled "File: New Untitled Text File", key
  `cmd+n`, menu File → "New File" (top of the menu, above "New Window"). Runs in a folder window and a
  single-file window. **Does not run in Welcome and when there is no window** (user's choice): the menu item is dimmed,
  the key does nothing. Reason: the Welcome window has no `Documents`, editor groups or hot exit; supporting it
  needs a new kind of window with no folder (and a new `WorkspaceKey`), nearly double the work, while the point of
  ⌘N is to create a file for the project that is open.
- Opens a pinned Untitled tab (not a preview) in the active editor group, with focus on it.
- Language: `files.defaultLanguage` if set (a language id, for example `"rust"`), otherwise Plain Text.
  Add this setting to `default-settings.json` with the value `""`.
- Tab name: the first non-empty line of the content, trimmed of whitespace at both ends, at most 40 characters (add `…` when
  cut); empty content gives `Untitled-N`. The tooltip / path bar shows `Untitled-N`.
- The tab icon follows the current language (Plain Text → the text-file icon).

### 4.3 Saving an Untitled

- `workbench.action.files.save` (⌘S), `:w` in Vim, and Save in the close-tab / quit dialog all
  go the Save As path when the tab is Untitled.
- The native save dialog (`cx.prompt_for_new_path`):
  - starting directory: the project's root directory; for a single-file window, the directory containing that file;
  - suggested name: the tab name (§4.2) with the characters `/ \ : * ? " < > |` and control characters removed, whitespace trimmed;
    if empty, `Untitled-N`. No extension is added automatically.
- Pressing Cancel: nothing changes, the tab stays dirty. If it was in a close-tab / quit flow, that flow stops
  as when Cancel is pressed in the Save / Don't Save / Cancel dialog.
- If the save succeeds: write the file through the existing save flow (`files.trimTrailingWhitespace`, `insertFinalNewline`,
  `files.eol`… as for a normal file; UTF-8 encoding without BOM). Then **that same tab** becomes a real file:
  `Documents::rename(untitled, target)` keeps undo, cursor, scroll; language, LSP, git and the watcher work as for
  a file just opened. The number `N` is returned.
- An Untitled shown in several editor groups (split screen): all views become the real file together (one
  document, `rename` moves every view).
- Saving outside the root directory, or in a single-file window: exactly as opening a file outside the directory does
  today (the window keeps its kind, nothing special added for this case). The spec writer's decision.
- A target that matches a file already open in another tab: the old document for that path is replaced with the content just saved
  (like VS Code: the macOS dialog already asked about Replace). The old tab for that path closes; the Untitled tab stays.
- A write error (permissions, disk full): reported like a normal save today, the tab stays Untitled and dirty.
- Format on save: runs according to the language of the **target path** if the existing format mechanism makes that easy;
  otherwise skip it for the first save (format does not exist yet, see the roadmap — note it when doing Format).

### 4.4 ⇧⌘S — Save As

- Command `workbench.action.files.saveAs`, titled "File: Save As...", key `shift+cmd+s`, menu File →
  "Save As…" (right under "Save"). Runs on the active code tab; other tabs (Search, diff…) do nothing.
- Untitled: as §4.3.
- A real file: the dialog starts in the file's directory, and the suggested name is the current file name. Saves the current
  content (including unsaved) to the target. **Every tab** showing that document (including in other groups) switches to the
  new file, keeping undo / cursor / scroll — using `Documents::rename`. The old file on disk is **unchanged** and
  is no longer open anywhere. A deliberate deviation from VS Code (VS Code only switches the active tab, other tabs keep the
  old file): following VS Code would require splitting one document into two, costly for a rare case. The spec
  writer's decision.
- Choosing exactly the current path: the same as a normal Save.

### 4.5 Untitled on close, quit and hot exit

- Autosave (every `files.autoSave` value) skips Untitled.
- Closing an Untitled tab (⌘W, the × button, Close Others…):
  - empty content: close without asking;
  - with content: ask Save / Don't Save / Cancel like a dirty file (`save.rs`, `CLOSE_ANSWERS`). Save goes
    §4.3.
- An Untitled with content always counts as dirty; an empty one does not.
- Quit / close window:
  - `files.hotExit` on (default): do not ask; an Untitled with content is backed up like a dirty file. On reopening
    the window: the Untitled tabs reappear **at the end of the editor group** (via the existing mechanism for opening backups with no tab,
    `workbench.rs` ~622–649), with the right `Untitled-N`, the right content, language, and still dirty.
    The old tab position is not preserved: doing so would mean writing Untitled into the list of tabs to reopen, against §4.1.
    The spec writer's decision.
  - `files.hotExit: "off"`: ask Save All / Don't Save / Cancel as now; Save All goes §4.3 through each Untitled in
    turn, and Cancel in any dialog stops the quit.
- Save All (`workbench.action.files.saveAll`): saves normal files as before, then asks where to save
  each Untitled with content in turn; Cancel on one skips that one and goes on.
- An Untitled's backup: the same format and current `BACKUP_VERSION`; the header records the fake path (enough to
  recognize an Untitled), with no original mtime / size. Restore does not read the disk and does not pass the path through
  `Documents::key()` the way a normal file does. The spec writer's decision.

### 4.6 Commands, keys, menu

| Command | Title | Key | Menu |
|---|---|---|---|
| `workbench.action.files.newUntitledFile` | File: New Untitled Text File | `cmd+n` | File → New File |
| `workbench.action.files.saveAs` | File: Save As... | `shift+cmd+s` | File → Save As… |

Add lines for the two keys to the `keys.rs` table (`table_covers_every_default_binding`). Update
`docs/keyboard-shortcuts.md`, `docs/features.md`, `docs/settings.md` (`files.defaultLanguage`),
`CHANGELOG.md` (`[Unreleased]` → Added).

### 4.7 Vim

`:w` / `:wq` / `:x` on an Untitled go §4.3 (`:wq`, `:x` only close after the save completes; on Cancel they do not
close). `:w name` is out of scope.

## 5. Left to the implementer

- Where to put `is_untitled` (suggestion: `documents.rs` or a small module of its own) and the concrete list of guard places
  — §4.1 is the minimum list; find more by grepping every place that uses a document path to touch the disk.
- `Documents::key()` currently turns a path relative to the root directory: Untitled must pass through it unchanged.
- How hot exit records the number `N` and the tab order (reuse the existing tab-restore mechanism if possible).

## 6. Testing

- Unit / gpui tests for: numbering `N` (the smallest unused, returned on close / save); the tab name from the first
  line; each guard in §4.1 (no `untitled:*` file is created on disk, git / LSP are not called);
  ⌘S on an Untitled with a fake dialog (returns a path / returns Cancel); Save As on a real file (the old file
  unchanged, the tab switches, undo remains); closing an empty Untitled does not ask, with content it asks; hot exit restores an
  Untitled; Save All with an Untitled.
- Manual: ⌘N → type → ⌘S → save `foo.rs` in the project → it has color, LSP, Explorer shows the file; ⇧⌘S a file
  → the old file stays intact; quit with an unsaved Untitled → reopen and it is intact.
