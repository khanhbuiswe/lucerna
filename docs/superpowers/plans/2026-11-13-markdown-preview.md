# Markdown preview — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Markdown preview drawn with GPUI: `.md` opens as a preview by default, ⌘⇧V toggles back and forth with the source, ⌘K V opens a preview to the side with synchronized scrolling, and it updates automatically when an agent writes the file; add an "Open in Browser" command for HTML.

**Architecture:** The parser and renderer shared with hover live in `markdown.rs` (adding tables, task lists, images, strikethrough, the source line of each root block). A new view `MarkdownPreview` draws each root block as one item of a `gpui::list`, reparses on a background thread 200 ms after the last edit, and keeps the document through a passive `CodeView`. Two new `TabItem`s (static preview, dynamic preview) go through the editor group model, session saving, `workbench.editorAssociations` and context keys like file tabs.

**Tech Stack:** Rust, GPUI (only `lucerna-workbench`), `pulldown-cmark` 0.13 (already present), `globset` via `lucerna_syntax::Associations` (already present). No new crates.

**Spec:** `docs/superpowers/specs/2026-11-13-markdown-preview-design.md` (read alongside; the § section numbers in the plan point back to the spec). Research: `docs/research/2026-11-13-markdown-preview.md`.

## Global Constraints

- Cargo: `export PATH="/opt/homebrew/opt/rustup/bin:$PATH"`.
- Before every build/test: `scripts/prune-target.sh`; if `target/release` exists and no other cargo is running (`pgrep -x cargo`), delete it. Keep free disk space ≥ 1.2 GB.
- Only `lucerna-workbench` uses gpui. No macOS-only assumptions outside platform code.
- **Add no crate to `Cargo.lock`.** The release binary grows ≤ 300 KB (ceiling 500 KB) compared to the baseline measured in Task 10. RAM: the preview compared with a source tab of the same file grows ≤ 10 MB.
- Limits: the preview does not parse files > 5 MB (`MAX_PREVIEW_BYTES = 5 * 1024 * 1024`); images > 20 MB (`MAX_IMAGE_BYTES = 20 * 1024 * 1024`) are not loaded. Hover keeps `MAX_MARKDOWN_BYTES = 100_000`.
- Update debounce: 200 ms (`REPARSE_DELAY`). Maximum content width 800 px, horizontal padding 32 px, vertical 24 px.
- Colors come only from the theme (`theme.ui.*`); there is no `surface_background`, use `elevated_surface_background`.
- Commands and settings use exactly VS Code's IDs as in spec §4.2, §4.5, §4.6, §5.6.
- The UI thread does not parse the preview's markdown and does not read image files (gpui loads images off the UI thread).
- Commit by path (`git commit -- <paths>`), trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Never rebase main; check `.git/MERGE_HEAD` before committing.
- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo deny check` are clean.
- Tests run with `cargo test -p lucerna-workbench --test <file>` (or `-p lucerna-syntax`); gpui tests use the helpers `init`/`open`/`draw` like `tests/open_files.rs`.

## Review Focus

- **An agent keeps overwriting the file while the preview is scrolled partway**: the preview does not jump back to the top, it only parses the last snapshot. Task 5 has `rewrite_keeps_reading_position` and `burst_of_writes_parses_last_once`.
- **A `.md` file with CRLF or no trailing `\n`**: the root block's lines are still correct, synchronized scrolling does not drift. Task 1 has `root_lines_crlf_and_no_trailing_newline`.
- **Opening a `.md` that already has a source tab in the group** (Explorer click, ⌘P): activate the existing tab, do not open another preview. Task 3 has `plain_open_reuses_source_tab`.
- **Closing an unsaved source tab while the file's static preview is still open**: no save prompt, the preview still shows the unsaved buffer. Task 4 has `closing_dirty_source_keeps_preview_quiet`.
- **A relative image with `..` escaping the workspace, or a symlink pointing outside**: not loaded, shows the alt text. Task 8 has `image_outside_root_is_blocked` (compared after `canonicalize`).

---

### Task 1: Parser — tables, task lists, images, strikethrough, root block lines

**Files:**
- Modify: `crates/lucerna-workbench/src/markdown.rs` (types `Block`/`Inline`, `MarkdownBlocks`, `parse_markdown`, `Builder`, `nth_code`, the `Renderer::block` branches)
- Modify: `crates/lucerna-workbench/src/language_servers/requests.rs:791-796` if it matches the `Block::List` pattern
- Test: `crates/lucerna-workbench/tests/markdown_parse.rs` (new)

**Interfaces:**
- Produces:
  - `pub const PARSE_OPTIONS: pulldown_cmark::Options` = `ENABLE_TABLES | ENABLE_STRIKETHROUGH | ENABLE_TASKLISTS | ENABLE_YAML_STYLE_METADATA_BLOCKS | ENABLE_PLUSES_DELIMITED_METADATA_BLOCKS | ENABLE_GFM`. Hover and preview both use it.
  - `Block::List { start: Option<u64>, items: Vec<ListItem> }` (replaces `ordered: bool`; `start.is_some()` means ordered).
  - `pub struct ListItem { pub checked: Option<bool>, pub blocks: Vec<Block> }`.
  - `Block::Table { aligns: Vec<Align>, head: Vec<Vec<Inline>>, rows: Vec<Vec<Vec<Inline>>> }`, `pub enum Align { None, Left, Center, Right }`.
  - `Block::Image { url: String, alt: String }` when a paragraph consists of exactly one image (ignoring whitespace).
  - `Block::Alert { kind: AlertKind, blocks: Vec<Block> }` for `> [!NOTE]` etc. (`AlertKind`: `Note | Tip | Important | Warning | Caution`).
  - `Inline::Strike(Vec<Inline>)`, `Inline::Image { url: String, alt: String }`.
  - `MarkdownBlocks::root_lines(&self) -> &[Range<u32>]` (parallel to `blocks()`, 0-based, end exclusive), `MarkdownBlocks::line_count(&self) -> u32`, `MarkdownBlocks::code_index_start(&self, root: usize) -> usize` (the `code_runs` index of the first code block in root block `root`).
  - `pub fn parse_markdown_document(source: &str, fallback_language: Option<&str>) -> MarkdownBlocks`: does not cut at 100 KB (the caller checks 5 MB). `parse_markdown` keeps its signature, cuts, then calls the same internal function.
- Front matter (metadata block) is dropped, not turned into a block. Raw HTML is still dropped.

- [ ] **Step 1: Write the failing tests** in `tests/markdown_parse.rs`:

```rust
use lucerna_workbench::markdown::*;

#[test]
fn table_with_alignment() {
    let md = parse_markdown_document("| a | b | c |\n|:--|:-:|--:|\n| 1 | **2** | |\n", None);
    let Block::Table { aligns, head, rows } = &md.blocks()[0] else { panic!() };
    assert_eq!(aligns, &[Align::Left, Align::Center, Align::Right]);
    assert_eq!(head.len(), 3);
    assert_eq!(rows[0][1], vec![Inline::Strong(vec![Inline::Text("2".into())])]);
    assert!(rows[0][2].is_empty());
}

#[test]
fn nested_task_list_and_start() {
    let md = parse_markdown_document("3. x\n4. y\n\n- [ ] a\n  - [x] b\n", None);
    let Block::List { start, .. } = &md.blocks()[0] else { panic!() };
    assert_eq!(*start, Some(3));
    let Block::List { start: None, items } = &md.blocks()[1] else { panic!() };
    assert_eq!(items[0].checked, Some(false));
    let Block::List { items: inner, .. } = &items[0].blocks[1] else { panic!() };
    assert_eq!(inner[0].checked, Some(true));
}

#[test]
fn block_and_inline_images() {
    let md = parse_markdown_document("![logo](img/a.png)\n\ntext ![i](b.svg) end\n", None);
    assert_eq!(md.blocks()[0], Block::Image { url: "img/a.png".into(), alt: "logo".into() });
    let Block::Paragraph(inl) = &md.blocks()[1] else { panic!() };
    assert!(inl.contains(&Inline::Image { url: "b.svg".into(), alt: "i".into() }));
}

#[test]
fn front_matter_hidden_and_alert() {
    let md = parse_markdown_document("---\ntitle: x\n---\n\n> [!NOTE]\n> hi\n\n~~old~~\n", None);
    assert!(matches!(md.blocks()[0], Block::Alert { kind: AlertKind::Note, .. }));
    let Block::Paragraph(inl) = &md.blocks()[1] else { panic!() };
    assert!(matches!(inl[0], Inline::Strike(_)));
}

#[test]
fn root_lines_cover_blocks() {
    let src = "# T\n\npara\nline2\n\n```rs\na\nb\n```\n\n- a\n\n  b\n- c\n";
    let md = parse_markdown_document(src, None);
    assert_eq!(md.root_lines(), &[0..1, 2..4, 5..9, 10..14]);
    assert_eq!(md.line_count(), 14);
}

#[test]
fn root_lines_crlf_and_no_trailing_newline() {
    let md = parse_markdown_document("# T\r\n\r\npara", None);
    assert_eq!(md.root_lines(), &[0..1, 2..3]);
    assert_eq!(md.line_count(), 3);
}

#[test]
fn code_index_start_counts_nested_code() {
    let md = parse_markdown_document("```\na\n```\n\n> ```\n> b\n> ```\n\n```\nc\n```\n", None);
    assert_eq!((0..3).map(|i| md.code_index_start(i)).collect::<Vec<_>>(), vec![0, 1, 2]);
}

#[test]
fn hover_still_cuts_at_limit() {
    let big = "a".repeat(MAX_MARKDOWN_BYTES + 10);
    assert!(matches!(&parse_markdown(&big, None).blocks()[0], Block::Paragraph(i) if matches!(&i[0], Inline::Text(t) if t.len() == MAX_MARKDOWN_BYTES)));
    assert!(matches!(&parse_markdown_document(&big, None).blocks()[0], Block::Paragraph(i) if matches!(&i[0], Inline::Text(t) if t.len() == MAX_MARKDOWN_BYTES + 10)));
}
```

- [ ] **Step 2: Run** `cargo test -p lucerna-workbench --test markdown_parse` → does not compile (types do not exist yet).

- [ ] **Step 3: Implement.** Use `Parser::new_ext(src, PARSE_OPTIONS).into_offset_iter()`; when a root block ends (frame depth returns to the root), record the byte `range.start..range.end` then convert to lines through a table of `\n` positions (start line = the line of the first byte; end line = the line of the last byte `range.end - 1` plus 1). `line_count` = number of `\n` + 1 if the file does not end with `\n`, otherwise = number of `\n`. The `MetadataBlock` tag is swallowed (no text emitted). `TaskListMarker(b)` sets `checked` on the open item. `BlockQuote(Some(kind))` → `Alert`. The `Image` tag inside a paragraph is collected into `Inline::Image` (alt = the child text joined); a paragraph with only one `Inline::Image` → `Block::Image`. Current renderer: update the `List` branch for `ListItem`/`start`, draw `Strike` with `strikethrough`, draw `Alert` like `Quote` with a bold title (`Note`, `Tip`, …); `Table` and `Image` temporarily draw an empty `div()` (Task 2 replaces them). `nth_code` walks `Alert`, `ListItem.blocks`; table cells contain no code.

- [ ] **Step 4: Run** `cargo test -p lucerna-workbench --test markdown_parse --test hover` → PASS (the old hover unchanged).

- [ ] **Step 5: Commit** `feat(markdown): tables, task lists, images and block lines in the parser`.

---

### Task 2: Per-block renderer with `MarkdownStyle`

**Files:**
- Modify: `crates/lucerna-workbench/src/markdown.rs` (the Rendering part)
- Modify: `crates/lucerna-workbench/src/hover.rs` (only if the signature of `render_markdown_with` changes — it should not)
- Test: `crates/lucerna-workbench/tests/hover.rs` (add), `tests/markdown_render.rs` (new)

**Interfaces:**
- Consumes: Task 1.
- Produces:
  - `pub struct MarkdownStyle { pub text_size: f32, pub line_height: f32 /* factor */, pub block_gap: f32, pub heading_sizes: [f32; 6], pub heading_rules: bool /* rule under h1,h2 */, pub code_scroll_x: bool, pub images: Option<ImageLookup> }`; `MarkdownStyle::hover()` recreates exactly the current values (13 px, gap 8, headings 18/16/15/14/14/14, no rules, no horizontal scroll, `images: None`); `MarkdownStyle::preview(font_size: f32, line_height: f32, images: Option<ImageLookup>) -> Self` with headings 2.0/1.5/1.25/1/1/1 × `font_size`, rule under h1/h2 using `border_variant`, 24 px above a heading, 12 px below.
  - `pub type ImageLookup = Rc<dyn Fn(&str /* url */, &str /* alt */, usize /* image index */, &App) -> AnyElement>`: the renderer calls it for each image when present; Task 8 provides it.
  - `pub fn render_block(blocks: &MarkdownBlocks, root: usize, style: &MarkdownStyle, on_link: LinkHandler, cx: &App) -> AnyElement` — one root block, code numbered from `code_index_start(root)`.
  - `render_markdown_with` keeps its signature, inside it uses `MarkdownStyle::hover()` and `render_block` for each block.
- Block styles per spec §4.6: a table with a 1 px `border_variant` border, a bold first row on `elevated_surface_background`, alignment per `Align`, the table scrolls horizontally on its own; a task list uses `ui::checkbox(checked, cx)` instead of a bullet (no click handler); a code block on `elevated_surface_background` (or the hover color currently in use if different — keep one color for both), rounded `GRID`, padding 12 px, with `code_scroll_x` use `overflow_x_scroll` and no line wrapping; a quote has a 3 px left bar in `border`, text `text_muted`.
- An image when `style.images` is `None`: a faint bordered frame, the icon `icons::IMAGE`, the alt text (`image_placeholder(alt, note: Option<&str>, cx) -> AnyElement`, `pub(crate)`, reused by Task 8).

- [ ] **Step 1: Failing tests.** In `tests/markdown_render.rs` (host like `tests/hover.rs`): `table_renders_cells` — draw `"| a | b |\n|--|--|\n| 1 | 2 |\n"` with `render_block(.., 0, &MarkdownStyle::preview(14., 1.6, None), ..)`, then `cx.debug_bounds("md-table-cell-1-1")` is `Some` (attach `debug_selector` to the cell `md-table-cell-{row}-{col}`, the first row is row 0). `task_list_draws_checkboxes` — `"- [x] a\n- [ ] b\n"` has `md-task-0` and `md-task-1`. `image_without_context_shows_alt` — has `md-image-alt`. In `tests/hover.rs`: `hover_layout_unchanged` — the popover height of the existing `DOC` equals the value measured before the change (record the measurement from Step 2 in the test).
- [ ] **Step 2: Run** `cargo test -p lucerna-workbench --test markdown_render --test hover` → FAIL (`render_block` does not exist yet). Before changing the renderer, measure the current popover height to fill into `hover_layout_unchanged`.
- [ ] **Step 3: Implement** as in Interfaces.
- [ ] **Step 4: Run again** → PASS.
- [ ] **Step 5: Commit** `feat(markdown): per-block renderer with hover and preview styles`.

---

### Task 3: Preview tabs in the editor group model

**Files:**
- Modify: `crates/lucerna-workbench/src/editor_groups/model.rs`
- Modify: `crates/lucerna-workbench/src/lib.rs` (re-export the new types)
- Test: `crates/lucerna-workbench/tests/editor_groups_model.rs`

**Interfaces:**
- Produces:
  - `TabItem::MarkdownPreview(Arc<Path>)` (static) and `TabItem::MarkdownPreviewDynamic(Arc<Path>)` (the file being followed).
  - `TabItem::is_singleton`: `true` for `MarkdownPreviewDynamic`, `false` for `MarkdownPreview`. A singleton is compared by kind: add `fn same_slot(&self, other: &TabItem) -> bool` (dynamic matches every dynamic; the rest `==`) and use it in `restore` and `open_singleton`.
  - `Tab::path()`: `Some` for `File` and `MarkdownPreview`; `None` for dynamic. `Tab::previewed_path() -> Option<&Arc<Path>>`: the path of both preview kinds. `Tab::title()`: `MarkdownPreview` = the file name; dynamic = `format!("Preview {name}")`.
  - `pub enum FileView { Source, Preview }`.
  - `EditorGroups::open_file(&mut self, path: Arc<Path>, view: FileView, mode: OpenMode, preview_enabled: bool, reuse_any: bool) -> TabId`. `open(path, mode, pe)` = `open_file(path, FileView::Source, mode, pe, false)`. `reuse_any`: if the target group already has `File(path)` or `MarkdownPreview(path)`, activate that tab (the active tab first, then by `mru`), do not create a new one; the pin/replace-preview-tab rules are exactly as in `open`.
  - `EditorGroups::replace_item(&mut self, id: TabId, item: TabItem) -> Option<TabId>`: replace tab `id` with a new tab in the same position, same `pinned`, active if the old tab was active; if the group already has a tab with `item`, close tab `id` and activate the other tab.
  - `EditorGroups::open_dynamic_preview(&mut self, path: Arc<Path>) -> TabId`: if a dynamic one exists anywhere, change its path and activate it (its group becomes active? **no**: keep the current active group, only set the active tab within its group); if none exists, open it pinned in the group to the right of the active group (create it if needed, like `ToSide`) then **return the active group to the old group** (focus stays in the editor).
  - `EditorGroups::retarget_dynamic(&mut self, path: Arc<Path>) -> bool`: change the dynamic's path if there is one and it differs.
  - `EditorGroups::dynamic_preview(&self) -> Option<(usize, &Tab)>`.
  - Session saving: `pub const MARKDOWN_PREVIEW_STATE_PREFIX: &str = "lucerna:markdown-preview:"`, `pub const MARKDOWN_PREVIEW_DYNAMIC_STATE_PREFIX: &str = "lucerna:markdown-preview-dynamic:"`; `saved_path`/`saved_item` join/split the prefix with the absolute path.

- [ ] **Step 1: Failing tests** (add to `tests/editor_groups_model.rs`; the helper `tabs()` changes to use `title()` or `previewed_path()` for preview tabs):
  - `plain_open_reuses_source_tab`: `open(a.md, Pinned)` then `open_file(a.md, Preview, Preview, true, true)` → one tab, `TabItem::File`, same id.
  - `plain_open_reuses_static_preview`: the other way around, the existing preview is activated.
  - `point_open_adds_source_next_to_preview`: with `MarkdownPreview(a.md)`, `open(a.md, Pinned)` → two tabs, the preview is still there.
  - `replace_item_keeps_slot_and_pin`: three tabs, replace the middle one (a preview tab, not pinned) → same position, `pinned == false`, active.
  - `replace_item_into_existing_closes_old`.
  - `dynamic_preview_is_singleton_and_keeps_focus`: `open_dynamic_preview(a.md)` then `(b.md)` → one dynamic tab in group 1, title `"Preview b.md"`, `active_group() == 0`.
  - `previews_saved_and_restored`: `state()` contains `"lucerna:markdown-preview:/x/a.md"` and `"lucerna:markdown-preview-dynamic:/x/b.md"`; `restore` gives back exactly the `TabItem`s; with two dynamic ones in the state only the first is kept.
  - `static_preview_has_path_dynamic_has_not`.
- [ ] **Step 2: Run** `cargo test -p lucerna-workbench --test editor_groups_model` → FAIL.
- [ ] **Step 3: Implement** as in Interfaces. Fix every `match` on `TabItem` in the crate so it compiles: `workbench.rs` tab body factory (temporarily `MarkdownPreview*` use the `File` branch of the path until Task 4), `editor_groups/view.rs` icon (`None` for now).
- [ ] **Step 4: Run** `cargo test -p lucerna-workbench --test editor_groups_model --test editor_groups_view --test workspace_state` → PASS.
- [ ] **Step 5: Commit** `feat(editor-groups): static and dynamic markdown preview tabs`.

---

### Task 4: The `MarkdownPreview` view — drawing, holding the document, the file's tab

**Files:**
- Create: `crates/lucerna-workbench/src/markdown_preview/mod.rs` (view), `crates/lucerna-workbench/src/markdown_preview/sync.rs` (leave empty until Task 6 if not needed yet)
- Modify: `crates/lucerna-workbench/src/lib.rs`, `src/workbench.rs` (tab body factory), `src/editor_groups/view.rs` (tab icon `icons::EYE`, `disk_bar` for a static preview), `src/icons.rs` + `crates/lucerna-workbench/assets/icons/eye.svg` (Lucide `eye`, ISC, like the other icons), `crates/lucerna-settings/assets/default-settings.json`
- Test: `crates/lucerna-workbench/tests/markdown_preview.rs` (new)

**Interfaces:**
- Consumes: Tasks 1–3.
- Produces:
  - `pub struct MarkdownPreview` (entity) with `MarkdownPreview::new(path: Arc<Path>, dynamic: bool, documents: Entity<Documents>, workbench: WeakEntity<Workbench>, cx: &mut Context<Self>) -> Self`.
  - `pub fn path(&self) -> &Arc<Path>`, `pub fn set_path(&mut self, path: Arc<Path>, cx)` (a dynamic one switching file: drop the old passive `CodeView`, open the new one, parse immediately).
  - `pub fn blocks(&self) -> Option<&Arc<MarkdownBlocks>>`, `pub fn list_state(&self) -> &ListState`.
  - `pub fn top_line(&self) -> f32` / `pub fn scroll_to_line(&mut self, line: f32, cx)` — declared in Task 4 with a simple implementation (go to the block containing the line, offset 0); Task 6 uses the full mapping.
  - Holding the document: `documents.open_preview(path)` → a passive `Entity<CodeView>`, kept in the struct, not drawn (spec §5.3 allows it).
  - States: no snapshot yet → an empty div; `DocState::Deleted`/error → the group's `disk_bar` (like a source tab, view.rs looks it up with `Tab::path()`); `len_bytes > MAX_PREVIEW_BYTES` → the notice "File too large to preview" + `text_button("Show Source")` running `markdown.showSource`.
  - Render: `list(list_state, |ix| render_block(..ix..))` in a centered frame at most 800 px wide, padding 32/24, background `editor_background`; `MarkdownStyle::preview(fontSize, lineHeight, ..)` reads `markdown.preview.fontSize` (14), `markdown.preview.lineHeight` (1.6), `markdown.preview.fontFamily` ("" = `UI_FONT_FAMILY`).
  - Default settings added to `default-settings.json`: `"markdown.preview.fontSize": 14`, `"markdown.preview.lineHeight": 1.6`, `"markdown.preview.fontFamily": ""`.
  - Tab body factory: `MarkdownPreview(path)` → new view (`dynamic: false`); `MarkdownPreviewDynamic(path)` → new view (`dynamic: true`). A dynamic tab has no dirty dot (because `path()` is `None`).

- [ ] **Step 1: Failing tests** in `tests/markdown_preview.rs` (helpers `init`/`open`/`draw` like `tests/open_files.rs`; add `open_preview(wb, path, cx) -> Entity<MarkdownPreview>` which opens `TabItem::MarkdownPreview` through the model then downcasts `active_body`):
  - `preview_renders_blocks`: a file with 3 blocks → `list_state().item_count() == 3`, `cx.debug_bounds("md-table-cell-0-0")` exists.
  - `closing_dirty_source_keeps_preview_quiet`: open a source tab, type (dirty), open a static preview in the same group, close the source tab → no prompt (use the way `tests/save.rs` checks prompts), the preview still has the block with the text just typed; the preview tab has a dirty dot (the existing dot `debug_selector`).
  - `too_large_file_shows_notice`: a file of 5 MB + 1 byte → `debug_bounds("md-preview-too-large")` exists, `blocks()` is `None`.
  - `deleted_file_shows_disk_bar`.
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3: Implement** (the first parse may be synchronous in `new` in this task; Task 5 moves it to the background).
- [ ] **Step 4: Run** `cargo test -p lucerna-workbench --test markdown_preview --test save --test editor_groups_view` → PASS.
- [ ] **Step 5: Commit** `feat(markdown-preview): preview view as a tab of its file`.

---

### Task 5: Debounced background update, keep the reading position

**Files:**
- Modify: `crates/lucerna-workbench/src/markdown_preview/mod.rs`
- Test: `crates/lucerna-workbench/tests/markdown_preview.rs`

**Interfaces:**
- Consumes: `DocumentEvent::{Opened, Edited}` of `Documents` (filtered to the exact `path`), `Documents::snapshot_of`, `revision_of`.
- Produces: `pub const REPARSE_DELAY: Duration = Duration::from_millis(200)`; `pub fn parses_started(&self) -> u64` (a counter, for tests); `pub fn refresh(&mut self, cx)` (drop the image cache, reparse immediately — Task 7 binds the command).
- Rules: the first time, parse immediately; an event arriving when no wait is pending schedules `REPARSE_DELAY` and then takes the snapshot **at that time**; if a wait is already pending, ignore it. Parse with `cx.background_spawn` on the snapshot's string; a result whose `revision` is older than the one being shown is dropped. Before replacing: `top = top_line()`; after `list_state.reset(n)`: `scroll_to_line(top)`. Changing theme/font only `notify`s.

- [ ] **Step 1: Failing tests:**
  - `rewrite_on_disk_updates_preview`: open the preview, `fs::write` new content, send `WorktreeEvent::Changed(Modified)` like `tests/external_changes.rs:153`, `cx.executor().advance_clock(REPARSE_DELAY)`, `draw` → the new block shows.
  - `burst_of_writes_parses_last_once`: 5 writes + events within 100 ms (advance 20 ms each) → after `advance_clock(REPARSE_DELAY)`: `parses_started()` increases by exactly 1 and the content is the 5th write.
  - `rewrite_keeps_reading_position`: a file with 200 headings (`# h{i}\n\npara\n\n`), scroll to block 120, rewrite the file with 1 new heading inserted at the top → the top block after the update is the block carrying `# h119` or `# h120` (by line), not 0.
  - `typing_updates_without_save`: type into the source tab in another group → the preview changes after the debounce.
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** `feat(markdown-preview): background reparse with debounce, keep position`.

---

### Task 6: Line ↔ scroll mapping and synchronized scrolling

**Files:**
- Modify: `crates/lucerna-workbench/src/markdown_preview/sync.rs`, `markdown_preview/mod.rs`
- Modify: `crates/lucerna-workbench/src/code_view/mod.rs` (scroll event)
- Modify: `src/workbench.rs` (connect editor ↔ preview of the same file), `default-settings.json`
- Test: `crates/lucerna-workbench/tests/markdown_sync.rs` (new, pure), `tests/markdown_preview.rs`

**Interfaces:**
- Produces (pure, `pub`):
  - `pub fn line_to_item(root_lines: &[Range<u32>], line_count: u32, line: f32) -> (usize, f32)` — the last block with `start ≤ line`; `fraction = (line − start) / max(1, next_start − start)` where `next_start` is the first line of the next block (or `line_count`), clamped to `[0, 1)`; before the first block → `(0, 0.)`; an empty list → `(0, 0.)`.
  - `pub fn item_to_line(root_lines: &[Range<u32>], line_count: u32, item: usize, fraction: f32) -> f32` — the inverse.
  - `CodeView` emits `pub enum CodeViewEvent { ScrolledTo(u32) }` when `scroll_top_row` changes due to the user or `set_scroll_top_row` (`impl EventEmitter<CodeViewEvent> for CodeView`), the line being the buffer line (folds already skipped).
  - `MarkdownPreview::top_line` / `scroll_to_line` use the two functions above with `list_state.logical_scroll_top()` and `bounds_for_item(ix)` (not yet measured → offset 0, fixed in a later frame through a pending flag).
  - Default settings: `"markdown.preview.scrollPreviewWithEditor": true`, `"markdown.preview.scrollEditorWithPreview": true`.
  - Wiring in `Workbench`: for each visible `MarkdownPreview` and each visible `CodeView` of the same `path` in **another group**: editor `ScrolledTo(row)` → `preview.scroll_to_line(row as f32)` if the setting is on; `list_state.set_scroll_handler` (runs only when the user scrolls) → `view.set_scroll_top_row(top_line().floor() as u32)` if the setting is on, set a `syncing` flag to drop exactly one echoed `ScrolledTo`.
  - Switching in place (Task 7 uses it): `pub(crate) fn source_line_for_preview(preview) -> u32` and opening a preview with an initial line.
- [ ] **Step 1: Failing tests** in `tests/markdown_sync.rs`: `before_first_block`, `inside_block_fraction` (`[0..2, 4..10]`, line 7 → `(1, 0.5)`), `blank_line_belongs_to_previous` (line 3 → `(0, 0.75)`), `after_last_block` (line 20, `line_count` 20 → `(1, ~0.999)`), `empty_doc`, `roundtrip` (with 50 fixed pseudo-random lines: `item_to_line(line_to_item(l)) ≈ l` with error < 1e-3). In `tests/markdown_preview.rs`: `editor_scroll_moves_side_preview` (group 0 source, group 1 preview; `set_scroll_top_row(150)` → the preview's top block contains line 150), `setting_off_stops_sync`.
- [ ] **Step 2: Run** `cargo test -p lucerna-workbench --test markdown_sync --test markdown_preview` → FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** → PASS; also `cargo test -p lucerna-workbench --test code_view` still PASS.
- [ ] **Step 5: Commit** `feat(markdown-preview): two-way scroll sync with the editor`.

---

### Task 7: Commands, keys, button, association, context keys

**Files:**
- Create: `crates/lucerna-workbench/src/markdown_preview/commands.rs`
- Modify: `src/workbench.rs` (`open_path`, new `open_source`, `open_path_at`, `navigate`, `context_keys`, command registration at ~1851), `src/editor_groups/view.rs` (a button at the right edge of the path bar), `crates/lucerna-syntax/src/associations.rs` (`matches` becomes `pub`, shared doc for every glob table), `crates/lucerna-settings/assets/default-keybindings.json`, `default-settings.json`, `src/app_menu.rs` if the View menu lists commands (add "Open Preview to the Side")
- Test: `tests/markdown_preview.rs`, `tests/open_files.rs`, `crates/lucerna-settings/tests/keymap.rs`

**Interfaces:**
- Produces:
  - `Workbench::open_source(&mut self, path: Arc<Path>, mode: OpenMode, cx)` — the old content of `open_path`. `open_path_at` and `navigate` call `open_source`.
  - `Workbench::open_path(path, mode, cx)` — looks up `workbench.editorAssociations` (`BTreeMap<String, String>`, paths relative to the root like `files.associations`) through `Associations::new(&map).matches(rel)`: the first value `"vscode.markdown.preview.editor"` → `open_file(path, FileView::Preview, mode, pe, true)`; `"default"` or no match → `open_file(path, FileView::Source, mode, pe, true)`. Record recent as before. Add a pure `pub fn editor_for(path: &Path, rel: &Path, map: &BTreeMap<String,String>) -> FileView` for testing.
  - Defaults: `"workbench.editorAssociations": { "*.md": "vscode.markdown.preview.editor", "*.markdown": "vscode.markdown.preview.editor" }`, `"markdown.preview.doubleClickToSwitchToEditor": true`.
  - Commands (`commands.rs`, `register_commands(&mut CommandRegistry)`): `markdown.togglePreview` "Markdown: Toggle Preview", `markdown.showPreview` "Markdown: Open Preview", `markdown.showPreviewToSide` "Markdown: Open Preview to the Side", `markdown.showSource` "Markdown: Show Source", `markdown.preview.refresh` "Markdown: Refresh Preview". Behavior exactly per the table in spec §4.2. Switching in place uses `replace_item` and keeps the position: source → preview: `scroll_to_line(view.scroll_top_row())`; preview → source: `go_to_and_scroll_when_opened(path, Point{row: line,0}, line)` with `line = top_line().floor()`. From a dynamic one, `togglePreview`/`showSource` open the source in the active group (`open_source`, Pinned). A command run when the active tab is not markdown does nothing.
  - Double-click on a block (setting on): static preview → `showSource` at `root_lines[ix].start`; dynamic → if a group has the file's `CodeView`, activate it and `go_to`; if not, `open_source` in the preview's group.
  - Dynamic follows the active tab: when the active tab of a group other than the one holding the dynamic one is a `File` or `MarkdownPreview` of a file whose language is `markdown` (`languages.language_for_file(rel, "")`), `retarget_dynamic(path)` and `set_path`, then `scroll_to_line` following the editor if there is one.
  - Context keys: `activeEditor` = `"markdownPreview"` when the active body is a `MarkdownPreview`; `editorLangId` and `resourceExtname` of a dynamic preview come from `MarkdownPreview::path()` (a static preview already has them through `Tab::path()`).
  - Keys (`default-keybindings.json`): `{ "key": "cmd+shift+v", "command": "markdown.togglePreview", "when": "editorLangId == markdown || activeEditor == 'markdownPreview'" }`, `{ "key": "cmd+k v", "command": "markdown.showPreviewToSide", "when": "editorLangId == markdown" }`. Check that `cmd+shift+v` is not already assigned to another command in the file; if it is, note it and put the markdown binding later (wins by order, as in VS Code).
  - Path bar button: `icon_button("md-preview-toggle-{gi}", icons::EYE, false, cx)` tooltip "Open Preview to the Side (⌘K V)", click → `markdown.showPreviewToSide`, Alt+click → `markdown.togglePreview`; on a preview: `icons::FILE_CODE`, tooltip "Show Source (⌘⇧V)", click → `markdown.togglePreview`. Only shown when the group's active tab is markdown. The keys in the tooltip come from the real keymap (like the other tooltips), not hard-coded.
  - Scroll keys in the preview: ↑/↓ (one line of text = `fontSize × lineHeight`), PageUp/Down, Space/Shift+Space, Home/End, ⌘↑/⌘↓ — handled in the view's `on_key_down`, no new commands.
- [ ] **Step 1: Failing tests:**
  - `tests/open_files.rs`: `md_opens_as_preview_by_default` (Explorer-style `open_path(a.md, Preview)` → a `MarkdownPreview` tab), `md_jump_opens_source` (`open_path_at(a.md, .., Point{row:10})` → `File`), `go_back_returns_to_source` (jump twice, `navigate(back)` → `File`), `association_default_opens_source` (user settings `{"workbench.editorAssociations": {"*.md": "default"}}`), `association_full_path_glob` (`"docs/**/*.md"` matches; `README.md` at the root does not), `rs_unaffected`.
  - `tests/markdown_preview.rs`: `toggle_swaps_in_place_and_back` (same tab position, same pinned, top line ±1 block), `show_to_side_keeps_focus_in_editor` (`active_group()` is still 0), `dynamic_follows_active_markdown` (open b.md in group 0 → the dynamic one is `Preview b.md`; open `x.rs` → still b.md), `double_click_opens_source_at_block`, `context_keys_in_preview` (`editorLangId == "markdown"`, `activeEditor == "markdownPreview"` for both static and dynamic), `cmd_shift_v_dispatches_toggle` (through the real keymap).
  - `crates/lucerna-settings/tests/keymap.rs`: `markdown_bindings_parse` (no `when` errors).
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p lucerna-workbench --test markdown_preview --test open_files --test nav_history --test workspace_state` and `cargo test -p lucerna-settings -p lucerna-syntax` → PASS.
- [ ] **Step 5: Commit** `feat(markdown-preview): commands, keys, path bar button and editor associations`.

---

### Task 8: Local images and links

**Files:**
- Create: `crates/lucerna-workbench/src/markdown_preview/resolve.rs`
- Modify: `src/markdown.rs` (draw images through `img()` when there is an `ImageContext`; the preview's `LinkHandler`), `markdown_preview/mod.rs` (`ImageContext`, image cache, scrolling to a slug), `default-settings.json`
- Test: `crates/lucerna-workbench/tests/markdown_resolve.rs` (new, pure), `tests/markdown_preview.rs`

**Interfaces:**
- Produces (pure):
  - `pub enum ImageSrc { Local(PathBuf), Blocked(&'static str) }`; `pub fn resolve_image(url: &str, ctx: &ImageContext) -> ImageSrc`: `http(s):`/`data:` → `Blocked("remote")`; `/x` relative to `ctx.root`; relative paths relative to `ctx.base_dir`; decode `%xx`; normalize `..` lexically; then `canonicalize` (error → `Blocked("missing")`); not under one of the elements of `ctx.allowed` (already canonicalized) → `Blocked("outside")`; `metadata.len() > MAX_IMAGE_BYTES` → `Blocked("too-large")`. This function touches the disk: Task 5's background parse job, after `parse_markdown_document`, calls it for every image URL and returns a `HashMap<String, ImageSrc>` along with the blocks; the view keeps that table and passes it into `MarkdownStyle::preview(.., Some(lookup))` through an `ImageLookup` that reads the table. It is not called while drawing.
  - `pub struct ImageContext { pub base_dir: PathBuf, pub root: PathBuf, pub allowed: Vec<PathBuf> }`.
  - `ctx.allowed` = `[root]` when the file is inside the root; `[the file's folder]` when it is not (single-file mode: the root is the file's folder).
  - `pub enum LinkAction { Web(String), Anchor(String), Markdown { path: PathBuf, slug: Option<String> }, Source { path: PathBuf, point: Point }, File(PathBuf), None }`; `pub fn resolve_link(url: &str, base_dir: &Path, root: &Path) -> LinkAction`: `http(s):`/`mailto:` → `Web`; `#slug` → `Anchor`; a relative path, `/x` or `file://` to `.md`/`.markdown` → with `#L…` it is `Source` (use the existing `line_fragment`), otherwise `Markdown`; another file → `File` (with `#L…` it is `Source`).
  - `pub fn heading_slugs(blocks: &MarkdownBlocks) -> Vec<(String, usize)>` (GitHub-style slug: lowercase, drop characters that are not letters/digits/whitespace/`-`, whitespace → `-`, duplicates get `-1`, `-2`; the value is the root block index).
  - Executing links in the preview: `Web` → `cx.open_url`; `Anchor` → scroll to the block; `Markdown` → `markdown.preview.openMarkdownLinks` (`"inPreview"` by default, added to `default-settings.json`) ⇒ static preview: `open_path(path, Preview)`, dynamic: `retarget_dynamic` + `set_path`; `"inEditor"` ⇒ `open_source`; with a `slug`, scroll after parsing; `Source` → `jump_to`; `File` → `open_path(.., Preview)`. Hover keeps the old `open_markdown_link`.
  - Images: `img(path).max_w_full()` keeping the aspect ratio, not enlarged beyond the real size, cached per view (a `RetainAllImageCache` attached to `MarkdownPreview`, released when the view drops; `refresh` clears it). `Blocked(reason)` → `image_placeholder(alt, note)` with the note `"Image too large"` for `too-large`, no note for the other reasons. A link's tooltip is the URL.
- [ ] **Step 1: Failing tests** `tests/markdown_resolve.rs` (use `TempDir`): `relative_image`, `root_absolute_image`, `percent_decoded`, `image_outside_root_is_blocked` (both `../../x.png` and a symlink inside the root pointing outside), `missing_image`, `remote_image_blocked`, `too_large_image` (a sparse file of 20 MB + 1 via `set_len`), `loose_file_allows_its_dir`; `link_kinds` (a table of URLs → `LinkAction`), `slugs_github_style` (`"Hello, World!"` → `hello-world`; two `"A"` → `a`, `a-1`; Vietnamese `"Tổng quan"` → `tổng-quan`). In `tests/markdown_preview.rs`: `local_image_drawn` (`debug_bounds("md-image-0")`), `anchor_link_scrolls`, `md_link_opens_preview_tab`.
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** `feat(markdown-preview): local images and links`.

---

### Task 9: "Open in Browser" for HTML

**Files:**
- Modify: `src/open_commands.rs` (register), `src/quick_open/` if the palette filters commands by condition
- Test: `tests/open_commands.rs`

**Interfaces:**
- Produces: the command `workbench.action.openInBrowser`, title "Open in Browser"; no key assigned. Running it: if the active tab has a `path()` with extension `html`/`htm` (case-insensitive) → `cx.open_url(url::Url::from_file_path(path))` (the on-disk version); otherwise → does nothing. The palette lists the command only when that condition holds: if the palette has no mechanism to hide commands by context yet, add to `CommandRegistry` a `fn register_when(id, title, when: &'static str, handler)` that evaluates `when` using `context_keys` (`resourceExtname == .html || resourceExtname == .htm`) and the palette drops commands whose `when` is false.
- [ ] **Step 1: Failing tests:** `open_in_browser_listed_only_for_html` (the palette has the command when the tab is `a.html`, not when it is `a.rs`), `open_in_browser_url_is_file_url` (split out the pure function `browser_url(path: &Path) -> Option<String>`; `/tmp/a b.html` → `file:///tmp/a%20b.html`).
- [ ] **Step 2: Run** → FAIL. **Step 3: Implement.** **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** `feat: Open in Browser for HTML files`.

---

### Task 10: Measure binary, RAM, performance and review the UI against Zed

**Files:**
- Modify: `docs/superpowers/specs/2026-11-13-markdown-preview-design.md` (add §10 "Measurements"), `tests/perf.rs` (add a parse bench)
- Modify: `docs/superpowers/specs/2026-11-08-lucerna-phase1-design.md` §1.3 (add a line "Markdown preview (done)")

- [ ] **Step 1: Baseline.** On the commit right before Task 1 (`git merge-base HEAD main` or the spec commit `d3c4043`): a temporary `git worktree add`, `scripts/bundle-app.sh`, record `stat -f %z .../luc`; delete the temporary worktree. Note: keep only one profile on disk.
- [ ] **Step 2: After.** `scripts/bundle-app.sh` on HEAD, record the size. The difference ≤ 300 KB; if > 500 KB then stop and find the cause (a new `image` feature, generic bloat) before merging.
- [ ] **Step 3: Crates.** `git diff <baseline> -- Cargo.lock` is empty.
- [ ] **Step 4: RAM.** The three cases A/B/C of spec §6 with `footprint <pid>` after 10 s; B − A ≤ 10 MB; C returns to near A after closing the tab.
- [ ] **Step 5: Parse.** Add `parse_5000_lines` to `tests/perf.rs` (5000 lines ~200 KB mixing headings/paragraphs/tables/code) — `parse_markdown_document` < 30 ms in release; run it the way `perf.rs` is run now.
- [ ] **Step 6: UI.** Open `docs/superpowers/specs/2026-11-08-lucerna-phase1-design.md` in Lucerna (preview) and Zed (`markdown::OpenPreview`) side by side; compare spacing, heading sizes, tables, code blocks, quotes, empty states; fix any difference that has no reason. Put the screenshots into the merge notes.
- [ ] **Step 7: Record the measurements** in spec §10 and commit `docs: markdown preview measurements`.

---

## Order and dependencies

1 → 2 → (3 can run in parallel with 1–2) → 4 → 5 → 6 → 7 → 8 → 9 (independent, can be done any time) → 10.
