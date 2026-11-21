# Markdown preview: design

Date: 2026-11-13. Status: approved.

## 1. Intent

Lucerna users let agents write the code, and agents write a lot of `.md`:
READMEs, plans, specs, notes. Reading those files as source is tiring: tables
turn into rows of `|`, task lists into `- [ ]`, images are just paths. Lucerna
needs a markdown preview **drawn with GPUI** to read them, updating as soon as
the agent rewrites the file.

Because reading is the main activity (`design-principles.md` §3), `.md` files
**open as a preview by default**. The source is one key away (⌘⇧V). Anyone who
wants it like VS Code turns it off with a setting.

Success: open a 2000-line spec written by an agent and read it right away with
its tables, task lists and images; when the agent overwrites the file, the
preview follows within half a second without losing the reader's place; the
binary grows by no more than a few hundred KB.

## 2. Goals, non-goals, anti-goals

**Goals**
- Open the preview in place (replacing the source tab) or in the group beside it.
- A `.md` file opens as a preview by default, unless jumping to a specific line.
- Updates itself when the buffer changes: typing, undo, and the file being
  overwritten on disk.
- Display: headings, paragraphs, emphasis, strikethrough, inline code,
  highlighted code blocks, lists (ordered, starting from any number), task
  lists, blockquotes, rules, tables (with alignment), local images, links.
- Two-way synchronized scrolling with the editor of the same file.
- An "Open in Browser" command for `.html`/`.htm` files.

**Non-goals (this version)**
- HTML preview (see §4.3).
- `http(s)` images (needs an HTTP client for gpui; the alt text is shown).
- Mermaid, math formulas, footnotes, raw HTML in markdown (raw HTML is
  ignored, as hover does today), table-style front matter (front matter is
  hidden).
- A "locked" preview (`markdown.showLockedPreviewToSide`,
  `markdown.preview.toggleLock`), clicking a checkbox to edit the file,
  search in the preview (⌘F), selecting and copying text in the preview,
  `markdown.styles`.

**Anti-goals** (the feature works but is still a failure if):
- The binary grows by more than 500 KB, or a new crate is added to `Cargo.lock`.
- The UI thread stalls when a large file changes continuously (the agent is writing).
- The preview jumps back to the top every time the file changes.
- A webview gets pulled in.

## 3. References

Details are in `docs/research/2026-11-13-markdown-preview.md`. Summary:

| Aspect | VS Code | Zed | Lucerna |
|---|---|---|---|
| Rendering | webview + markdown-it | GPUI elements, pulldown-cmark, background parse | **Zed** (GPUI, pulldown-cmark, background parse) |
| Keys | ⌘⇧V `markdown.togglePreview` (in place, press again to return to source), ⌘K V `showPreviewToSide` | ⌘⇧V opens / closes inside the preview, ⌘K V to the side | **VS Code** |
| Button | editor title, Alt = in place | Eye icon on the pane toolbar, Alt = split | **Zed** (icon on the path bar), click behavior follows VS Code |
| Preview to the side | dynamic: follows the active md editor | a separate Follow command | **VS Code** (dynamic) |
| Opening md as preview | `workbench.editorAssociations` (off by default) | `open_markdown_files_in_preview` (off by default) | **VS Code**'s key, **on by default** (a deviation, §4.1) |
| Updating | throttle 300 ms | debounce 200 ms via the buffer | via the buffer, 200 ms |
| Sync | by scroll position, two-way, interpolated by line | by cursor and click | **VS Code** (by scroll) |
| Building the document | DOM | one element for the whole document | `gpui::list`, one item per root block (better than Zed for performance) |
| Task list | not in core | clickable checkbox | display-only checkbox |
| Images | in the workspace or the file's folder | relative to the file, `/x` relative to the worktree | like Zed, with VS Code's limits |

## 4. Decisions

### 4.1 Open `.md` as a preview by default

Uses VS Code's `workbench.editorAssociations` setting (an object of glob →
editor id). Lucerna understands only one editor id:
`vscode.markdown.preview.editor` and the special id `default` (source).
Lucerna's default value:

```json
"workbench.editorAssociations": { "*.md": "vscode.markdown.preview.editor", "*.markdown": "vscode.markdown.preview.editor" }
```

The glob matches by file name (no `/`) or by absolute path (with `/`), like
VS Code. The user deletes the key or sets `"*.md": "default"` to go back to
the source.

This deviates from VS Code (whose default is empty) because Lucerna users read
markdown more than they edit it.

Which way of opening goes where:

| Way of opening | Result for a file matching the association |
|---|---|
| Click in the Explorer, ⌘P (no `:line`), Open Recent, `luc file.md`, session restore, Open File… | preview |
| Search results, Go to Definition, References, diagnostics, ⌘P with `:line`, link `file#L10`, Go Back/Forward | source (the exact line is needed) |

Rule in code: `open_path_at` currently calls `open_path`, and Go Back
(`Workbench::navigate`) calls `open_path` and then
`go_to_and_scroll_when_opened`. So split two entry points: one function that
opens the **source** (used by `open_path_at`, `jump_to`, `navigate` and every
place that has a destination), and `open_path` (no destination), which looks
up the association and then calls that function or opens the preview. A
preview tab (italic, `workbench.editor.enablePreview`) applies just like a
file tab.

**A file already open in the group** (VS Code: reuse the editor already open):
- Opening with no destination: if the group already has a source tab or a
  static preview of the file, activate that tab (prefer the active tab, then
  the most recently used one), and do not create a new tab. If there is none,
  follow the association.
- Opening with a destination: use the file's source tab if the group already
  has one; if not, open a new source tab. A static preview of that file that
  is already open is left as it is.

**The preview is a tab of the file.** `Tab::path()` returns the path for
`MarkdownPreview` too, so the preview takes part everywhere that relies on it:
the "unsaved" dot, the save prompt when closing the last tab of an unsaved
file, automatic pinning of the preview tab of an unsaved file, `disk_bar`,
Explorer highlighting, recent files. ⌘S and auto-save while the preview has
focus save that file's buffer. The preview shows the buffer, including the
unsaved part. Closing the last source tab of an unsaved file while the file's
preview is still open does **not** prompt (the file still has a tab), the same
as closing one of two source tabs of the same file today. A dynamic preview
does not count as a tab of the file (it keeps changing files): `path()`
returns `None`, no dot, no save prompt.

### 4.2 Commands, keys, buttons

Commands (VS Code IDs, available in the command palette):

| Command | Title | What it does |
|---|---|---|
| `markdown.togglePreview` | Markdown: Toggle Preview | In an md source: replace that tab with a preview in place. In an in-place preview: replace it with the source. In a side preview: open that file's source in this group. |
| `markdown.showPreview` | Markdown: Open Preview | Open a preview in place (no toggling back). |
| `markdown.showPreviewToSide` | Markdown: Open Preview to the Side | Open (or reuse) the dynamic preview in the group to the right; create the group if there is none. Focus stays in the editor. |
| `markdown.showSource` | Markdown: Show Source | From a preview, open the source in place. |
| `markdown.preview.refresh` | Markdown: Refresh Preview | Parse again. |
| `workbench.action.openInBrowser` | Open in Browser | §4.3. |

Default keys (added to `default-keybindings.json`):

```json
{ "key": "cmd+shift+v", "command": "markdown.togglePreview", "when": "editorLangId == markdown || activeEditor == 'markdownPreview'" },
{ "key": "cmd+k v", "command": "markdown.showPreviewToSide", "when": "editorLangId == markdown" }
```

A new context key `activeEditor` (value `markdownPreview` when the active tab
is a preview, static or dynamic). Today `editorLangId` comes from the active
`CodeView` or `tab.path()`; this needs fixing so that when a preview is
active, `editorLangId` is `markdown` (the language of the file being
previewed) and `resourceExtname` is that file's extension, so that ⌘K V works
from the preview. `when.rs` already has `||`, `==` and quoted strings.

Keys in the preview (with focus in the preview): ↑/↓ scroll one line of text,
PageUp/Down, Space/Shift+Space one page, Home/End or ⌘↑/⌘↓ to the start/end,
like scrolling in the editor. Vim mode does not apply to the preview (this
version).

**Button** (UI after Zed): the path bar (below the tab bar) has an
`icon_button` at the right edge:
- A `.md` source tab: Lucide icon `eye` (not yet in
  `crates/lucerna-workbench/assets/icons`: take `eye.svg` from the Lucide
  repo and add it to `icons.rs` like the other icons), tooltip "Open Preview
  to the Side (⌘K V)". Click → `showPreviewToSide`; Alt+click →
  `togglePreview` (like VS Code `navigation@1`, Alt = in place).
- A preview tab: Lucide icon `file-code`, tooltip "Show Source (⌘⇧V)". Click →
  `togglePreview`.

### 4.3 HTML: only "Open in Browser"

There is no HTML preview. The command `workbench.action.openInBrowser` ("Open
in Browser") opens the active file in the default browser (`cx.open_url` with
a `file://` URL). The command shows in the palette only when the active tab is
an `.html`/`.htm` file (`resourceExtname`), with no key bound. For an unsaved
file it opens the version on disk (no note needed in the tooltip/palette).

Why we do not build an HTML preview with a webview (`wry` / WKWebView):
- A native window would have to be attached to the GPUI view tree: z-order,
  focus, clipping when scrolling or splitting groups all have to be handled
  ourselves.
- Linux needs WebKitGTK and Windows needs WebView2: a system dependency per
  platform, contrary to Lucerna going to Linux and Windows.
- Each webview costs an extra 50–100 MB of RAM, several times the whole editor.
- The user's browser already does this better, DevTools included.

VS Code and Zed also have no built-in HTML preview.

### 4.4 In-place preview and side preview

Two new kinds of tab in `TabItem`:

- `MarkdownPreview(Arc<Path>)`: a **static** preview of one file. Created by
  an association, `togglePreview` or `showPreview`. Tab title: the file name
  (`README.md`), and the icon is the `eye` icon instead of the file icon.
  Each group has at most one such tab per path; it can coexist with the
  source tab of the same file. It can be a preview tab (italic). It is not a
  singleton.
- `MarkdownPreviewDynamic`: a preview that **follows** the active markdown
  editor, created by `showPreviewToSide`. Title: `Preview README.md`. At most
  one in the whole window (a singleton): calling it again changes the file
  and activates that tab. When the active tab of another group is the source
  or a static preview of a different markdown file, the dynamic preview
  switches to that file, keeping its position matched to the editor
  (synchronized scrolling). A non-markdown tab leaves it unchanged.

`togglePreview` replaces the tab in place: the new tab sits exactly where the
old tab was, and keeps the pinned state. The reading position is kept across
the switch: the preview opens scrolled to the place matching the editor's top
line, and vice versa (using the mapping in §5.4).

Session state (`workspace_state`): saved like the other special tabs, as the
relative strings `lucerna:markdown-preview:<absolute path>` and
`lucerna:markdown-preview-dynamic:<absolute path>`. Restoring reopens the
right kind of tab; if the file no longer exists, the tab is dropped.

### 4.5 Synchronized scrolling

Following VS Code, two settings (default `true`):
- `markdown.preview.scrollPreviewWithEditor`: scroll the editor → the preview scrolls.
- `markdown.preview.scrollEditorWithPreview`: scroll the preview → the editor scrolls.

They apply only between the preview and the **visible** editor of the same
file in another group (the ⌘K V case). An in-place preview has no editor
showing at the same time, so it uses the mapping only when switching back and
forth (§4.4).

`markdown.preview.doubleClickToSwitchToEditor` (Lucerna default `true`;
newer VS Code defaults to `false`): double-clicking a block in the preview →
opens the source at that block's first line (in place if the preview is
static; in the group that has the file's editor if it is dynamic, in place if
there is none). This deviates from VS Code because, with the preview as the
default, it is the fastest way to edit exactly where you are reading.

`markdown.preview.markEditorSelection` is not built in this version.

### 4.6 Typography and layout

Follows Zed for the look: `editor_background` background, UI text
(`UI_FONT_FAMILY`), code uses the editor font. The content is centered, at
most 800 px wide, with 32 px horizontal padding and 24 px vertical padding.
Typography settings follow VS Code: `markdown.preview.fontSize` (default 14),
`markdown.preview.lineHeight` (1.6), `markdown.preview.fontFamily` (default
empty = UI font).

The style of each block (colors from the theme, no hard-coded colors):
- Heading: font size h1 2.0×, h2 1.5×, h3 1.25×, h4–h6 1×; bold; h1 and h2
  have a `border_variant` rule underneath. 24 px above, 12 px below.
- Code block: `elevated_surface_background` background (the theme has no
  `surface_background`; if hover already uses a different color for code,
  share that color), `GRID` corner radius, 12 px padding, scrolls
  horizontally for long lines (no wrapping).
- Blockquote: a 3 px left bar in `border`, `text_muted` text.
- Table: 1 px `border_variant` border, a bold first row with the same
  background as the code block, aligned per `:--`/`:-:`/`--:`, columns shrink
  to their content; a table wider than the frame scrolls horizontally on its
  own.
- Task list: `ui::checkbox` (already in `ui.rs`) instead of the bullet,
  not clickable.
- Image: at most as wide as the content frame, keeping its aspect ratio, not
  enlarged beyond its real size.

Hover keeps its current font size and spacing: the preview passes its own set
of style parameters into the renderer (§5.2).

## 5. Technical design

### 5.1 Parser (`markdown.rs`, shared with hover)

Extend `parse_markdown`, keeping the hover API:
- Enable `ENABLE_TABLES | ENABLE_STRIKETHROUGH | ENABLE_TASKLISTS |
  ENABLE_YAML_STYLE_METADATA_BLOCKS | ENABLE_PLUSES_DELIMITED_METADATA_BLOCKS
  | ENABLE_GFM` (metadata is enabled only to hide front matter; `GFM` is for
  `> [!NOTE]` alerts, shown as a blockquote with a bold title). Hover uses
  the same flags.
- `Block` gains: `Table { aligns, head: Vec<Vec<Inline>>, rows:
  Vec<Vec<Vec<Inline>>> }`, `Image { url, alt, title }` (when a paragraph has
  only one image: a block image), a task marker on list items
  (`items: Vec<ListItem>` with `checked: Option<bool>`), a list with a start
  number (`start: Option<u64>`).
- `Inline` gains `Strike(Vec<Inline>)`, `Image { url, alt }` (an inline image).
- Each **root block** (a direct child of the document) carries its source
  line range `lines: Range<u32>` (0-based, the last line excluded), taken from
  `Parser::into_offset_iter()` and then converting byte offsets → lines with a
  table of `\n` positions built once. Nested blocks do not need lines.
- `MAX_MARKDOWN_BYTES` (100 KB) stays for hover. The preview uses its own
  limit of **5 MB**; anything larger makes the preview show the message "File
  too large to preview" with a Show Source button, without parsing.

### 5.2 Renderer

Split `Renderer` into a function that renders **one block** with a
`MarkdownStyle` (base font size, line height, block spacing, heading sizes,
maximum image width). Hover calls it for every block inside one `div` as it
does today; the preview calls it for each item of the list. Code block
highlighting reuses the cached `code_runs`. Links use the existing
`LinkHandler`, plus the rules in §5.6.

### 5.3 The `MarkdownPreview` view (new `markdown_preview.rs`)

- Holds the current `Arc<MarkdownBlocks>`, the `ListState` of `gpui::list`
  (one item per root block, `ListAlignment::Top`, just enough overdraw), the
  file path and the kind (static or dynamic).
- Keeps the document open in `Documents` when no editor of the file exists:
  `Documents` needs a way to "hold" a document without going through
  `CodeView` (when the handle is dropped, the document is reclaimed as a view
  closes). The specific way is up to the implementer; holding a passive,
  undrawn `CodeView` is acceptable if it is more compact. This way a file
  that the agent overwrites is still reloaded and the preview can still
  listen.
- Loading state: document loading → an empty frame (no spinner in the first
  200 ms); read error / file deleted → use the same message and `disk_bar` as
  the source tab.

### 5.4 Updating

- Subscribe to `DocumentEvent::Edited` (and `Opened`) of the exact path. The
  first time, parse immediately; after that, coalesce: if a parse is already
  pending, skip; otherwise schedule 200 ms later and parse the **latest**
  snapshot at that moment.
- Parsing runs on a background thread (`cx.background_spawn`) with an
  immutable `BufferSnapshot`; the old result is displayed until the new one
  arrives. A result that arrives later than a newer one is dropped (compare
  `revision`).
- Keep the reader's place when swapping blocks: before swapping, take the top
  block (`logical_scroll_top`) and the corresponding source line (§5.5);
  after swapping, `ListState::reset` and then scroll to the block that
  contains that line at the same fraction within the block. When the agent
  overwrites the entire file, the position is still kept by line.
- Theme, font, `languages_generation` changes: just redraw (the runs cache is
  already keyed by generation).
- Image changes on disk: not watched in this version; `markdown.preview.refresh`
  reloads (clears the view's image cache).

### 5.5 Line ↔ scroll position mapping

Pure functions, testable without gpui, placed in `markdown.rs` or
`markdown_preview/sync.rs`:

- `line → (item, fraction)`: the last block with `lines.start ≤ line`;
  `fraction = (line − start) / max(1, next_start − start)` where `next_start`
  is the start line of the next block (or the document's line count), clamped
  to [0, 1). A blank line between two blocks belongs to the earlier block.
- `(item, offset_px, item_height) → line` (a real number): the reverse,
  `start +
  offset/height × (next_start − start)`.

Editor → preview: when the `CodeView` of the same file (visible, another
group) changes `scroll_top_row`, the preview does `scroll_to(ListOffset {
item_ix, offset_in_item: fraction × item height })`. If the item height is not
yet measured, use 0 and fix it on the next frame. Preview → editor: when the
`ListState` scrolls because of the user, compute the line and then
`set_scroll_top_row(floor)`. Loop prevention: the receiving side sets a
"scroll caused by sync" flag and ignores its own scroll event until the next
frame. No extra throttling: at most once per frame. (The scroll handler of
`gpui::list` runs only when the user scrolls with the mouse/trackpad, not
when `scroll_to` is called, so the editor → preview direction does not cause
a loop by itself; the flag is mainly needed for the preview → editor
direction.)

`CodeView` needs an event (or observe) that reports `scroll_top_row`
changing; if there is none yet, add it without changing current behavior. A
folded line: use the real buffer line, not the displayed line.

### 5.6 Images and links

Images (gpui's `img()`, which already reads png/jpg/gif/webp/svg/bmp/tiff/ico;
no crate added):
- `http(s):`, `data:` → not loaded; show the alt text in a dim-bordered frame
  with the Lucide `image` icon (already present).
- A relative path → relative to the md file's folder; starting with `/` →
  relative to the workspace root (single-file mode: the file's folder).
  Decode `%xx`, normalize `..` lexically.
- Only images inside the workspace root are loaded, or inside the md file's
  folder if the file is outside the workspace (like VS Code's
  `localResourceRoots`). Outside that area or nonexistent → the alt-text frame
  as above.
- The image cache is per view (`RetainAllImageCache` or equivalent), released
  when the tab closes. An image over 20 MB (file size) is not loaded; the alt
  text is shown with "Image too large".

Links: today `link_target` accepts only absolute URLs (`url::Url::parse`), so
relative links and `mailto:` both slip through, and `open_markdown_link`
always calls `jump_to`. The preview needs a link-resolving function **with a
base path** (the md file being viewed) that returns the kind of target; the
preview's `LinkHandler` holds that path in its closure, and hover keeps the
old behavior. The rules:
- `http(s):`, `mailto:` → the default browser / app.
- `#slug` → scroll to the heading with that slug (GitHub-style slug:
  lowercase, punctuation removed, whitespace → `-`, duplicates get `-1`,
  `-2`).
- A relative path or `file://` to a `.md`/`.markdown` file → open the preview
  (`markdown.preview.openMarkdownLinks`, default `inPreview`; `inEditor`
  opens the source). Opens **like a click in the Explorer** (`open_path`,
  italic tab), without switching the file inside the tab being viewed as VS
  Code does, because Lucerna has no in-tab history for the preview. From a
  dynamic preview it changes its own file (it follows a file anyway). With
  `#L10` → open the source at that line (rule in §4.1); with `#slug` → open
  the preview and scroll to the heading.
- Any other file → open like `open_path` (an Explorer click).
- Hovering a link: a hand cursor; the URL shows in the tooltip.

### 5.7 Performance

- The UI thread does not parse: parsing is in the background, and the UI only
  draws the items currently visible.
- Target: a 5000-line file (about 200 KB) parses in under 30 ms on the
  background thread; scrolling holds 120 fps on an M1 like the editor; one
  update when the agent writes the file does not drop a frame.
- A long code block (over 2000 lines) is still one item; highlighting uses
  `SYNC_PARSE_BUDGET` as hover does, and past it draws plain text.

## 6. Measuring binary size and RAM

**Current baseline**: the release `luc` of 2026-11-13 is **48,543,136 bytes**
(`/Applications/Lucerna.app/Contents/MacOS/luc`). Re-measure the baseline on
the commit right before starting, with the same profile and the same
`scripts/bundle-app.sh`.

**How to measure the binary**
1. Build release with `scripts/bundle-app.sh` on the base commit and the
   feature's last commit; compare `stat -f %z` of `luc`.
2. `cargo tree -p lucerna --prefix none -e normal | sort -u` before and after:
   there must be no new crates.
3. Budget: **grow ≤ 300 KB**, hard ceiling 500 KB. If exceeded, find the cause
   (for example a new `image` feature being turned on) before merging.

No new dependency needed: `pulldown-cmark` is already there, the table/task
list flags are just code inside that crate; `image` and `resvg` already come
in through gpui.

**How to measure RAM** (RSS via `footprint <pid>` or `ps -o rss`, 10 s after opening):
- A: open the Lucerna project, one source tab `docs/superpowers/specs/2026-11-08-lucerna-phase1-design.md`.
- B: like A but that tab is a preview.
- C: like B plus an md file with 3 PNG images of 1–2 MB.
- Budget: B − A ≤ 10 MB. C grows by roughly the decoded image size
  (width × height × 4 bytes) and returns close to the A level after the tab
  is closed.

Record the measurements at the end of this spec or in the merge notes.

## 7. Testing

- Parser: tables (alignment, empty cells, inline content in cells), nested
  task lists, lists starting from 3, block and inline images, hidden front
  matter, GFM alerts, the line range of root blocks (including code fences,
  multi-paragraph lists, a file ending with no `\n`, CRLF). Hover still comes
  out as before (the existing tests pass).
- Line ↔ scroll mapping: edge cases (before the first block, after the last
  block, a one-line block, an empty document), the two directions are
  approximately inverses of each other.
- Association: file-name glob, path glob, `default`, a deleted key; the "which
  way of opening goes where" table in §4.1.
- Editor group model: `togglePreview` replaces at the right position and
  pinned state; the dynamic preview is a singleton and changes file with the
  active tab; session restore.
- Updating: overwrite a file on disk (like the agent) → the preview changes
  within 500 ms and keeps the block being read; many consecutive writes parse
  only the last one.
- Images: relative, `/x`, `..` escaping the workspace is blocked, `%20`,
  nonexistent file, `http(s)` → alt.
- Links: `#slug`, `.md` opens the preview, `#L10` opens the source, external URL.
- Open in Browser: shown only for `.html`/`.htm`.
- UI: put it next to Zed (Zed's preview of the same file) and compare spacing,
  font size, borders, background, empty state (`design-principles.md` §1.1).

## 8. Accepted limitations

- **Go Back/Forward does not record a position in the preview.** `nav_here`
  records only when the active tab is a `CodeView`. Jumping from the preview
  (double-click back to the source, clicking a link) does not record the
  starting point, so Go Back does not return to the preview. Adding the tab
  kind and the preview scroll position to `NavEntry` is left for later.
- `Documents::discard` (Don't Save on close) drops the document even when a
  view remains. With the rule in §4.1, the prompt appears only when closing the
  last tab of the file (source or static preview), so no preview loses its
  document.

## 9. Left to the implementer

- How `Documents` holds the document for the preview (§5.3).
- How `CodeView` reports a `scroll_top_row` change (§5.5).
- The file structure in `lucerna-workbench` (one file or a `markdown_preview/`
  folder).
- The overdraw of `gpui::list` and how to measure item height the first time.
- The order of tasks in the plan; suggestion: parser → shared renderer → view
  and tab → association → synchronized scrolling → images/links → Open in
  Browser → measurement.

## 10. Measurements

Measured on 2026-11-13, same `release` profile, built with `cargo build --release -p lucerna`
(not `scripts/bundle-app.sh`, because it assumes `target/` inside the repo, while the target dir
is shared between worktrees). The baseline is `7b79e1b` (only the spec and plan), the feature head is `b7f54ef`.

| Item | Result |
|---|---|
| `luc` baseline `7b79e1b` | 48,792,336 bytes |
| `luc` feature head `b7f54ef` | 49,215,760 bytes |
| Difference | **+423,424 bytes (413 KB)**: over the 300 KB budget, under the 500 KB hard ceiling |
| `git diff 7b79e1b HEAD -- Cargo.lock` | empty, no new crates |
| Parse `parse_markdown_document`, 5014 lines, 191 KB | median 2.2 ms, max 3.0 ms (budget 30 ms), `tests/perf.rs::parse_5000_lines` |
| RAM A: source tab (a `.txt` copy of the phase 1 spec) | footprint 107 MB (RSS 113.6 MB) |
| RAM B: the same content opened as a preview (`.md`) | footprint 109 MB (RSS 117.4 MB), B − A = +2 MB, within the 10 MB budget |
| RAM C: an md file with 3 PNG images of 700×700 (1.4 MB each) | footprint 124 MB (RSS 130.2 MB), +15 MB over B, decoded images about 5.9 MB |

Notes: each case is a separate `luc <file>` process, measured after 10 s (`footprint <pid>`).
The CLI can open only one file, so C has no B tab to start from; "returns close to A after closing the tab"
was not measured and needs a manual run: open C, close the tab, wait 10 s, measure again.

Cause of the 413 KB difference (comparing `size -m` and `nm -m` of the two binaries, symbol names demangled with
`c++filt`, grouped by module): `__text` +207 KB, the unwind/exception tables (`__eh_frame`,
`__gcc_except_tab`, `__unwind_info`) +62 KB, `__DATA_CONST` +16 KB, and about +115 KB is
the symbol table in `__LINKEDIT` (the binary is not stripped; +936 symbols, name table +106 KB). Within
`__text`, functions whose names contain `markdown_preview`/`MarkdownPreview` take 118 KB: the
module's code (+54 KB: the view, `ScrollSync`, `follow_link`, `resolve_link`, commands) plus the
gpui generics instantiated for `MarkdownPreview` (`Context::spawn`/`listener`/`subscribe`,
`Div` drop glue). The parser/renderer extension in `markdown.rs` adds about 28 KB
(`parse` +13 KB, `Renderer::block` +10 KB). The rest is inlining reshuffled between crates
(for example `lsp_types` −15 KB, `serde_json::value` +19 KB), summing to nearly zero.
`pulldown_cmark` did not grow (−2.6 KB), there is no new crate or feature, and no
generic was seen being unusually duplicated, so there is no optimization to do. It is over the 300 KB budget but under the
500 KB ceiling: the user must accept it before merging.
