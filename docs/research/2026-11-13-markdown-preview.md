# Research: Markdown preview

Date 2026-11-13. Sources:
- VS Code: `microsoft/vscode` branch `main` (commit `08e8bbf`, 2026-11-13), folder `extensions/markdown-language-features/` (`package.json`, `package.nls.json`, `src/preview/*.ts`, `src/markdownEngine.ts`, `src/commands/*.ts`, `src/util/resources.ts`, `preview-src/*.ts`).
- Zed: the pinned GPUI revision `b5cd310b3b54dfc659fcc06453e6358843dfe216` (2026-11-07), crates `markdown_preview`, `markdown`, `mermaid_render`, `gpui`, `zed/src/zed/quick_action_bar/preview.rs`, `assets/keymaps/default-macos.json`, `assets/settings/default.json`. Zed is mostly GPL: only the design is recorded, no code is copied.

## 1. VS Code

### 1.1 Commands, keys, menus
Commands (`package.json` → `contributes.commands`, implemented in `src/commands/`):

| Command | Title | Notes |
|---|---|---|
| `markdown.showPreview` | Open Preview | opens the preview in the same editor group |
| `markdown.showPreviewToSide` | Open Preview to the Side | opens in the adjacent group, focus stays in the editor |
| `markdown.showLockedPreviewToSide` | Open Locked Preview to the Side | preview locked to this file |
| `markdown.showSource` | Show Source | from the preview back to the source editor |
| `markdown.preview.refresh` | Refresh Preview | |
| `markdown.preview.toggleLock` | Toggle Preview Locking | |
| `markdown.showPreviewSecuritySelector` | Change Preview Security Settings | |
| `markdown.reopenAsPreview` / `markdown.reopenAsSource` | Open as Preview / Open as Source | use `reopenActiveEditorWith` |
| `markdown.togglePreview` | Toggle Preview | replaces the source tab with the preview or vice versa, **in the same tab** |

Keys (latest on `main`):
- `cmd+k v` (`ctrl+k v`) → `markdown.showPreviewToSide`, `when`: `editorFocus && editorLangId =~ /^(markdown|prompt|instructions|chatagent|skill)$/ && !notebookEditorFocused`.
- `cmd+shift+v` (`ctrl+shift+v`) → **`markdown.togglePreview`** (the old version was `markdown.showPreview`), `when`: `!terminalFocus && ((editorFocus && resourceLangId =~ markdown… ) || activeCustomEditorId == 'vscode.markdown.preview.editor')`. That is, pressing ⌘⇧V in the editor switches to the preview, and pressing it in the preview goes back to the source.

Menus:
- `editor/title`, group `navigation@1`: the `markdown.showPreviewToSide` button (icon `$(open-preview)`), `alt` = `markdown.showPreview` (holding Alt when clicking opens in place). Shown only for markdown editors.
- `editor/title` when the preview is active: `markdown.showSource` (icon `$(file-code)`), and in the "…" menu (group `1_markdown`): Refresh, Toggle Lock, Security.
- `explorer/context` and `editor/title/context`: Open Preview.

### 1.2 What the preview is
- The preview is a **webview** (`markdown.preview`), the HTML is rendered by markdown-it and sent into the webview; the script `preview-src/index.ts` runs in the webview and handles scrolling, clicks, links.
- There is also a **custom editor** `vscode.markdown.preview.editor` (`contributes.customEditors`, priority `option` for `*.md, *.markdown, …`). Because it is a custom editor, "Reopen Editor With…" and `workbench.editorAssociations` work with it (e.g. `{"*.md": "vscode.markdown.preview.editor"}` to open md straight into the preview). This kind is a **static preview** (`StaticMarkdownPreview`), always tied to one file.
- `src/preview/previewManager.ts`: a preview opened by command is a **dynamic preview**. If not locked, it **follows the active markdown editor** (`onDidChangeActiveTextEditor` → `update(uri, line currently visible)`). A locked preview (`locked`) keeps its file. Toggle Lock switches back and forth; the tab title becomes "[Preview] x.md" when locked.
- Reopening a preview of the same file in the same column reuses the old preview (`matchesResource(resource, column, locked)`).

### 1.3 Content updates
`src/preview/preview.ts`:
- Listens to `workspace.onDidChangeTextDocument` (typing, no need to save) and `onDidOpenTextDocument`.
- Has a `FileSystemWatcher` on the source file; used only when VS Code is **not** holding that document open (to avoid a double refresh). So changes on disk (e.g. the agent writing the file) are picked up too.
- `refresh()`: **the first time it renders immediately**, later calls are batched by a **300 ms** timer (`#delay = 300`). It is not a "wait until typing stops" debounce but a throttle: the first call sets a timer, calls within 300 ms are ignored.
- Local images also have their own watcher (`#updateImageWatchers`): an image changed on disk → refresh.
- When the webview is hidden and its HTML is stale, it re-renders when shown.
- The webview updates by diffing the DOM (keeping loaded images), not by replacing the whole page.

### 1.4 Scroll sync
Settings (`contributes.configuration`, defaults on `main`):
- `markdown.preview.scrollPreviewWithEditor` (true): scrolling the editor → the preview scrolls along.
- `markdown.preview.scrollEditorWithPreview` (true): scrolling the preview → the editor scrolls along.
- `markdown.preview.markEditorSelection` (false on `main`; the old version defaulted to true): marks the line with the cursor in the preview (`activeLineMarker.ts`, adds a class to the element).
- `markdown.preview.doubleClickToSwitchToEditor` (false on `main`): double-click in the preview → opens the source editor at that line (`didClick` → `showSource` + set selection + `revealRange(InCenterIfOutsideViewport)`).

How lines are mapped:
- `markdownEngine.ts` has the plugin `pluginSourceMap`: every block token with a `token.map` gets the class `code-line` and `data-line="<start line, 0-based>"`.
- The webview (`preview-src/scroll-sync.ts`) builds a table of `[element, line]` from every `.code-line` (skipping `UL/OL` because child items have the same line; a code fence also counts the end line by the number of `\n`). The table is cached by the document version.
- Editor → preview: `topmostLineMonitor.ts` listens to `onDidChangeTextEditorVisibleRanges` (throttle 50 ms), takes the top line (with a fractional part), and sends it into the webview. The webview finds the elements before/after that line and **linearly interpolates** by `offsetTop` to get the scroll position. It waits for images to finish loading before scrolling.
- Preview → editor: the `scroll` event (throttle 50 ms) → from `scrollY` find the two elements bracketing it → interpolate the real line → `revealLine` → the extension scrolls the editor to that line (`scrollEditorToLine`, supports fractional lines).
- Loop prevention: each side has an "is scrolling because of the other side" flag (`#isScrolling` 200 ms in the extension; `scrollDisabledCount` 50 ms in the webview).
- The cursor line (`onDidChangeTextEditorSelection`) is sent separately for `markEditorSelection`.

### 1.5 Rendering
- markdown-it with `html: true`, highlighting by highlight.js. Settings: `markdown.preview.breaks` (false), `markdown.preview.linkify` (true, `fuzzyLink: false`), `markdown.preview.typographer` (false).
- Typography: `markdown.preview.fontFamily` (system-ui…), `markdown.preview.fontSize` (14), `markdown.preview.lineHeight` (1.6). `markdown.styles`: a list of extra CSS URLs/paths to inject. Extensions can contribute CSS (`markdown.previewStyles`), scripts and markdown-it plugins (`markdown.markdownItPlugins`).
- Tables: markdown-it core has GFM tables, strikethrough. **Task lists are not in core**: `- [ ]` shows up as text; to get checkboxes you must install an extension (e.g. "Markdown Checkboxes"), and those checkboxes do not edit the file. Mermaid and KaTeX also come through extensions.
- Front matter: `markdown.preview.frontMatter` = `hide | codeBlock | table` (default `table`), with a command "Configure Frontmatter Visibility".
- Headings have ids (GitHub-style slugs) so `#anchor` links work.

### 1.6 Images and security
- Relative image paths are resolved against the folder containing the md file, then turned into a webview URI (`asWebviewUri`). `/abs` resolves against the workspace root.
- `localResourceRoots` (`src/util/resources.ts`): if the file belongs to the workspace it is **every workspace folder**; otherwise it is **the folder containing the file**. Images outside these roots cannot be loaded.
- CSP (`documentRenderer.ts`), four levels chosen through "Change Preview Security Settings", stored in `globalState` per workspace (not a setting):
  - **Strict** (default): images/media/fonts/styles only from `self`, the webview, `https:`, `data:`; scripts only with VS Code's nonce. No script runs in the document, no `http:` images.
  - Allow insecure local content: adds `http://localhost:*`, `http://127.0.0.1:*`.
  - Allow insecure content: adds `http:`.
  - Disable: turns CSP off, lets scripts run.
- When content is blocked, the preview shows a "Some content has been disabled in this document" bar, and clicking it opens the security level picker.

### 1.7 Links
- `http(s):`, `mailto:` open in the system browser. `#anchor` scrolls within the preview.
- Relative links: the webview sends `openLink` to the extension. `markdown.preview.openMarkdownLinks` = `inPreview` (default) | `inEditor`: a link to another `.md` file opens **in the preview** (the dynamic preview switches to that file, scrolling to the fragment); other files open in the editor. Supports `file.md#L10`.

### 1.8 HTML
VS Code **has no built-in HTML preview**. It has Simple Browser (`simpleBrowser.show`, enter a URL) and Microsoft's "Live Preview" extension (`ms-vscode.live-server`) which runs a local server and then opens it in Simple Browser.

## 2. Zed

### 2.1 Structure
- The crate `markdown_preview` is down to 3 files: `markdown_preview.rs` (action declarations, `init`), `markdown_preview_settings.rs`, `markdown_preview_view.rs` (~5000 lines, mostly tests). The parser and renderer **have been merged into the shared crate `markdown`** (`markdown.rs`, `parser.rs`, `html/`, `mermaid.rs`) — also the crate that renders hover, the agent panel… The old files `markdown_parser.rs`, `markdown_renderer.rs`, `markdown_elements.rs` are no longer at this revision.
- `MarkdownPreviewView` is a workspace item (a tab in a pane), serializable (saves the file path + mode in the DB, restored on reopen).

### 2.2 Actions, keys, button
- `zed_actions::preview::markdown::{OpenPreview, OpenPreviewToTheSide}`; in the crate: `markdown::OpenFollowingPreview`, `CloseAndReturnToEditor`, `ScrollPageUp/Down`, `ScrollUp/Down`, `ScrollUpByItem/DownByItem`, `ScrollToTop/Bottom`.
- macOS keymap:
  - context `Editor && extension == md`: `cmd-k v` → `OpenPreviewToTheSide`, `cmd-shift-v` → `OpenPreview`.
  - context `MarkdownPreview`: `pageup/pagedown`, `up/down`, `alt-up/alt-down` (by block), `cmd-up/cmd-down`, `cmd-shift-v` → `CloseAndReturnToEditor` (⌘⇧V becomes a toggle key, like VS Code's `togglePreview`).
- Button: `quick_action_bar/preview.rs` adds an `IconButton` with an **Eye** icon on the pane toolbar when the item is an md file (also used for SVG and CSV). Tooltip "Preview Markdown" with the key; clicking = `OpenPreview`, **Alt+click = open in a split** (`OpenPreviewToTheSide`).
- The editor's right-click menu and the tab menu have "Open Markdown Preview" (`editor/src/mouse_context_menu.rs`, `editor/src/items.rs`).

### 2.3 Modes
- `MarkdownPreviewMode::Default`: tied to one specific editor (like VS Code's locked).
- `MarkdownPreviewMode::Follow` (`OpenFollowingPreview`): observes the workspace and **follows the active markdown editor** (like the dynamic preview).
- `OpenPreview` adds the preview to the **same pane** (re-activating it if it already exists). `OpenPreviewToTheSide` adds it to the adjacent pane (`adjacent_pane_of`, creating a split if there is none), without focusing the preview but **returning focus to the editor**.
- Setting `markdown_preview.open_markdown_files_in_preview` (false): opening an md file opens the preview; if the editor tab has no unsaved changes, that tab is replaced.
- Other settings: `font_size`, `font_family`, `code_font_family`, `heading_font_weight` (600), `limit_content_width` (true) + `max_width` (800 px, centered). Font size can be changed with the shared zoom keys when the preview has focus. There is a `markdown_preview_theme` in the theme settings (the preview uses a different theme than the editor).

### 2.4 Content updates
- Subscribes to editor events: `Edited`, `BufferEdited`, `BuffersEdited`, `DirtyChanged` → debounced update; `FileHandleChanged` (rename/move) → updates immediately, changing the root folder.
- Debounce `REPARSE_DEBOUNCE = 200 ms`, of the "if a task is already waiting, skip" kind (like VS Code's 300 ms throttle). The first open does not wait.
- No separate file watch: changes on disk go through the buffer (the buffer reloads itself when the file changes externally) → `BufferEdited`.
- Takes the buffer's whole text → `Markdown::reset(source)`. The `markdown` crate **parses on a background thread** (`cx.background_spawn`), keeping the old result until done, then `notify`. Skipped if the source is unchanged. Code block highlighting uses Zed's language registry (tree-sitter).

### 2.5 Rendering and scrolling
- Rendering: a `div` with `overflow_y_scroll` + `ScrollHandle`, holding **one `MarkdownElement`** that builds the whole document. **It does not use a virtualized `list`/`ListState`** at this revision (an older version of the crate used `list`). It has its own scrollbar per the editor settings, padding, a width limit.
- There is search in the preview (`SearchableItem`), text selection, a right-click menu with "Copy", "Copy as Markdown", "Copy Link".

### 2.6 Sync with the editor
Zed **does not sync by scroll position**, but by **cursor** and **byte offset**:
- The parser records `root_block_starts`: the starting byte offset of each root block (paragraph, heading, list, table…).
- Editor → preview: `EditorEvent::SelectionsChanged` → take the last selection's offset → find the root block containing that offset → (1) mark the active block with a **left bar** (`show_root_block_markers`, color `border` for the active block, `border_variant` for other blocks), (2) if the editor or the preview has focus, `request_autoscroll_to_source_index` scrolls the preview to that block (waiting for the parse to finish if one is running).
- Preview → editor: **a single click** on a block (`on_source_click`) → place the editor cursor at the source offset of the click spot, `Autoscroll::center`, without moving focus. Scrolling the preview does **not** scroll the editor.
- `CloseAndReturnToEditor` closes the preview and focuses the editor.

### 2.7 Rendering features
- pulldown-cmark 0.13, `PARSE_OPTIONS` = `TABLES | FOOTNOTES | OLD_FOOTNOTES | STRIKETHROUGH | TASKLISTS | SMART_PUNCTUATION | HEADING_ATTRIBUTES | PLUSES_DELIMITED_METADATA_BLOCKS | GFM | SUPERSCRIPT | SUBSCRIPT`; adds `YAML_STYLE_METADATA_BLOCKS` when `render_metadata_blocks`. It deliberately does not enable `MATH`, `DEFINITION_LIST`, `WIKILINKS` (a test forces each new option to be considered).
- The preview turns on `MarkdownOptions { parse_html, render_mermaid_diagrams, parse_heading_slugs, render_metadata_blocks }`.
- **Task list**: the checkbox (`ui::Checkbox`) is **clickable**; the callback replaces exactly the `[ ]` ↔ `[x]` span in the buffer via `editor.edit` (with a check of the old text), then refreshes the preview. It edits the buffer, the file is not saved.
- **Tables**: yes, including alignment.
- **HTML in markdown**: parsed with `html5ever`, supporting only a set of tags: `img` (src, alt, width, height), `p`, `h1–h6`, `ul/ol/li`, `blockquote`, `table/tr/th/td` (colspan, rowspan, align), `b/strong/i/em/del/ins/a`, a bit of `style` (underline, italic, bold, text-align) and `align`. No script runs, no arbitrary CSS.
- **Mermaid**: yes, the crate `mermaid_render` (based on `merman`, exports SVG then draws it with gpui's svg renderer); has zoom.
- **Front matter**: YAML `---` and `+++` render as a metadata block.
- Heading slug: a `#heading` link scrolls to the heading.
- Code block: highlighting, a Copy button shown on hover.

### 2.8 Images and links
- `image_resolver` (`resolve_preview_image`):
  - `data:` → skipped.
  - `http(s)://` → `Resource::Uri` (gpui loads it via `cx.http_client()`).
  - `/x` or `\x` → computed from the first worktree root; a relative path → from the md file's folder; `..` is normalized lexically. `%20` is decoded.
  - Remote project (SSH) → `image_store::project_image_source` reads through the project.
  - Returns an image only when the file exists.
- Uses a `RetainAllImageCache` attached to the view so images are not reloaded every frame.
- Link click (`handle_url_click`): `#slug` scrolls within the preview; a link to another md file opens **in the preview** (reusing the preview that already exists for that buffer), with `file.md#L10:5` → open and place the cursor; other files open in the editor (`open_url_or_file`); external URLs → the browser. Hovering a link shows the URL in the bottom corner.

### 2.9 HTML files
Zed **has no HTML preview** (only previews for Markdown, SVG, CSV/TSV — `svg_preview`, `tabular_data_preview`).

### 2.10 gpui: images
- `gpui/Cargo.toml` already depends on `image`, `resvg`, `usvg`. The workspace enables `image` 0.25 with the features `bmp, dds, exr, ff, gif, hdr, ico, jpeg, png, pnm, qoi, rayon, tga, tiff, webp`; `resvg` 0.46 with `text, system-fonts, memmap-fonts, raster-images`.
- `gpui::ImageFormat`: Png, Jpeg, Webp, Gif (animated, `decode_gif_image`), Svg, Bmp, Tiff, Ico, Pnm. `img()` accepts `ImageSource::Resource(Path | Uri | Embedded)`, `Image`, `Render`, or a closure.
- Lucerna's `Cargo.lock` already has `image 0.25.10`, `resvg/usvg 0.46.0`, `pulldown-cmark 0.13.6` (already used in `crates/lucerna-workbench/src/markdown.rs` for hover). Adding local images **pulls in no extra crate**. `http(s)` images need an `HttpClient` for gpui; Lucerna has not set one yet, so external URLs will not load unless one is added.

## 3. Similarities and differences

| Aspect | VS Code | Zed |
|---|---|---|
| Rendering technique | webview HTML + markdown-it | native GPUI elements + pulldown-cmark |
| Keys | ⌘⇧V toggles in place, ⌘K V to the side | ⌘⇧V opens (in the preview: closes, back to the editor), ⌘K V to the side |
| Button | editor title, Alt = open in place | pane toolbar (Eye), Alt = open in a split |
| Follows the active editor | dynamic (default) / locked | Default (locked) / Follow (separate command) |
| Open md straight into the preview | `workbench.editorAssociations` | `open_markdown_files_in_preview` |
| Updates | 300 ms throttle, file and image watchers | 200 ms debounce through the buffer, background parse |
| Sync | by **scroll position** both ways, interpolating by line | by **cursor** (editor → preview) and **click** (preview → editor), by offset |
| Line marking | `markEditorSelection` (off by default) | left bar on the active block (always on) |
| Task list | not in core | yes, clickable, edits the buffer |
| Mermaid | via extension | built in |
| HTML in md | full, subject to CSP | small tag set |
| Security | CSP, 4 levels | not needed (no scripts) |
| `.md` links | open in the preview (`openMarkdownLinks`) | open in the preview |
| Front matter | hide / codeBlock / table | metadata block |
| HTML preview | none (Simple Browser, Live Preview) | none |

## 4. Suggestions for Lucerna

Following `docs/design-principles.md`: behavior, settings, commands, keys follow VS Code; UI and performance follow Zed.

1. **Commands and keys (VS Code)**: `markdown.showPreview`, `markdown.showPreviewToSide`, `markdown.showLockedPreviewToSide`, `markdown.showSource`, `markdown.togglePreview`, `markdown.preview.toggleLock`, `markdown.preview.refresh`. ⌘⇧V = `togglePreview` (in place, press again to go back to the source — both the new VS Code and Zed make ⌘⇧V the toggle key). ⌘K V = to the side, focus stays in the editor. `when` based on the markdown language.
2. **Dynamic/locked (VS Code)**: a preview opened by command follows the active md editor; "Locked" keeps the file. Skip the security selector (there are no scripts).
3. **Button (Zed UI)**: an Eye icon on the editor group's bar, shown only for md files; click = to the side (like VS Code `navigation@1`), Alt = in place. The preview tab is named "Preview x.md". Reuse components from `ui.rs`.
4. **Rendering (Zed)**: native GPUI elements, no webview. Extend `crates/lucerna-workbench/src/markdown.rs` (which exists for hover) into a renderer shared by hover and preview, as Zed merged it into the `markdown` crate. Parse in the background, keep the old result until done, skip if the text is unchanged. Store the offset/start line of each root block while parsing (pulldown-cmark `into_offset_iter`).
5. **Updates**: listen to buffer edit events, throttle about 200–300 ms, render immediately the first time. Changes on disk should go through the buffer as in Zed (need to check that Lucerna's buffer-reload flow on external file changes emits an event the preview can hear); this matters because the agent writes files. If the preview is open with no buffer (locked, tab closed) a file watcher is needed as in VS Code.
6. **Sync**: at minimum like Zed (cursor → scroll the preview to the block + left bar; click on a block → place the editor cursor). VS Code's settings `markdown.preview.scrollPreviewWithEditor` / `scrollEditorWithPreview` need a mapping of top line ↔ block and interpolation by laid-out block height; worth doing in a later step, because Lucerna users mostly **read** files the agent wrote (more scrolling than typing). `doubleClickToSwitchToEditor` is easy once the mapping exists.
7. **Performance (Zed + better than Zed)**: Zed builds the whole document in one element. For long md files (logs, agent-generated plans), use `gpui::list` with each root block as an item so only the visible part is laid out; `ListState` also has a ready API to scroll to an item, which fits the block mapping.
8. **Features**: pulldown-cmark with TABLES, STRIKETHROUGH, TASKLISTS, FOOTNOTES, GFM (the `> [!NOTE]` alert), HEADING_ATTRIBUTES, YAML metadata. `breaks`, `linkify`, `typographer` follow the VS Code settings (`markdown.preview.*`; typographer ↔ `ENABLE_SMART_PUNCTUATION`, linkify has to be done by hand or with the `linkify` crate, breaks = treat SoftBreak as HardBreak). Front matter follows `markdown.preview.frontMatter` (default `table`, can be hidden). Task list: show checkboxes; clicking to edit the buffer like Zed is a small bonus that can wait. Mermaid, math, raw HTML: out of the first scope; inline HTML is shown only as text or ignored.
9. **Images**: gpui's `img()` already reads png/jpg/gif/webp/svg/bmp/tiff/ico, no extra crate. Resolve: relative to the md file's folder, `/x` against the workspace root, normalize `..`, decode `%xx`; restricted to the workspace folder (like `localResourceRoots`) or the folder containing the file if the file is outside the workspace. Use `RetainAllImageCache` or an equivalent. `http(s)` images need an HttpClient: later, show the alt text for now.
10. **Links**: `#anchor` scrolls to the heading (needs GitHub-style slugs); `.md` opens in the preview per `markdown.preview.openMarkdownLinks` (default `inPreview`); other files open in the editor; `#L10` places the cursor; external URLs open in the browser.
11. **Typography settings**: `markdown.preview.fontFamily`, `fontSize` (14), `lineHeight` (1.6) following VS Code; add a centered content width limit like Zed (`max_width` 800) because it looks tidier. `markdown.styles` (CSS) does not apply.
12. **HTML preview**: neither has one; Lucerna does not need one either.
