# Markdown preview — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Preview markdown vẽ bằng GPUI: `.md` mở mặc định ra preview, ⌘⇧V đổi qua lại với mã nguồn, ⌘K V mở preview bên cạnh có cuộn đồng bộ, tự cập nhật khi agent ghi file; thêm lệnh "Open in Browser" cho HTML.

**Architecture:** Parser và renderer dùng chung với hover nằm ở `markdown.rs` (thêm bảng, task list, ảnh, gạch ngang, dòng nguồn của mỗi khối gốc). View mới `MarkdownPreview` vẽ mỗi khối gốc thành một item của `gpui::list`, parse lại trên luồng nền 200 ms sau lần sửa cuối, giữ document qua một `CodeView` passive. Hai `TabItem` mới (preview tĩnh, preview dynamic) đi qua model nhóm editor, lưu phiên, `workbench.editorAssociations` và context key như tab file.

**Tech Stack:** Rust, GPUI (chỉ `lucerna-workbench`), `pulldown-cmark` 0.13 (đã có), `globset` qua `lucerna_syntax::Associations` (đã có). Không thêm crate.

**Spec:** `docs/superpowers/specs/2026-11-13-markdown-preview-design.md` (đọc kèm; số mục § trong plan trỏ về spec). Nghiên cứu: `docs/research/2026-11-13-markdown-preview.md`.

## Global Constraints

- Cargo: `export PATH="/opt/homebrew/opt/rustup/bin:$PATH"`.
- Trước mỗi build/test: `scripts/prune-target.sh`; có `target/release` và không có cargo khác chạy (`pgrep -x cargo`) thì xoá nó. Giữ đĩa trống ≥ 1.2 GB.
- Chỉ `lucerna-workbench` dùng gpui. Không giả định riêng macOS ngoài code nền tảng.
- **Không thêm crate nào vào `Cargo.lock`.** Binary release tăng ≤ 300 KB (trần 500 KB) so với mốc đo ở Task 10. RAM: preview so với tab mã nguồn cùng file tăng ≤ 10 MB.
- Giới hạn: preview không parse file > 5 MB (`MAX_PREVIEW_BYTES = 5 * 1024 * 1024`); ảnh > 20 MB (`MAX_IMAGE_BYTES = 20 * 1024 * 1024`) không tải. Hover giữ `MAX_MARKDOWN_BYTES = 100_000`.
- Debounce cập nhật: 200 ms (`REPARSE_DELAY`). Bề rộng nội dung tối đa 800 px, padding ngang 32 px, dọc 24 px.
- Màu chỉ lấy từ theme (`theme.ui.*`); không có `surface_background`, dùng `elevated_surface_background`.
- Lệnh và setting dùng đúng ID của VS Code như spec §4.2, §4.5, §4.6, §5.6.
- Luồng UI không parse markdown của preview và không đọc file ảnh (gpui tải ảnh ngoài luồng UI).
- Commit theo đường dẫn (`git commit -- <paths>`), trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Không rebase main; kiểm tra `.git/MERGE_HEAD` trước khi commit.
- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo deny check` sạch.
- Test chạy bằng `cargo test -p lucerna-workbench --test <file>` (hoặc `-p lucerna-syntax`); test gpui dùng helper `init`/`open`/`draw` giống `tests/open_files.rs`.

## Review Focus

- **Agent ghi đè file liên tục trong lúc preview đang cuộn giữa chừng**: preview không nhảy về đầu, chỉ parse snapshot cuối. Task 5 có `rewrite_keeps_reading_position` và `burst_of_writes_parses_last_once`.
- **File `.md` có CRLF hoặc không có `\n` cuối**: dòng của khối gốc vẫn đúng, cuộn đồng bộ không lệch. Task 1 có `root_lines_crlf_and_no_trailing_newline`.
- **Mở `.md` đang có tab mã nguồn trong nhóm** (click Explorer, ⌘P): kích hoạt tab có sẵn, không mở thêm preview. Task 3 có `plain_open_reuses_source_tab`.
- **Đóng tab mã nguồn chưa lưu khi preview tĩnh của file còn mở**: không hỏi lưu, preview vẫn hiện buffer chưa lưu. Task 4 có `closing_dirty_source_keeps_preview_quiet`.
- **Ảnh tương đối có `..` thoát khỏi workspace, hoặc symlink trỏ ra ngoài**: không tải, hiện alt. Task 8 có `image_outside_root_is_blocked` (so sánh sau `canonicalize`).

---

### Task 1: Parser — bảng, task list, ảnh, gạch ngang, dòng khối gốc

**Files:**
- Modify: `crates/lucerna-workbench/src/markdown.rs` (kiểu `Block`/`Inline`, `MarkdownBlocks`, `parse_markdown`, `Builder`, `nth_code`, các nhánh `Renderer::block`)
- Modify: `crates/lucerna-workbench/src/language_servers/requests.rs:791-796` nếu khớp mẫu `Block::List`
- Test: `crates/lucerna-workbench/tests/markdown_parse.rs` (mới)

**Interfaces:**
- Produces:
  - `pub const PARSE_OPTIONS: pulldown_cmark::Options` = `ENABLE_TABLES | ENABLE_STRIKETHROUGH | ENABLE_TASKLISTS | ENABLE_YAML_STYLE_METADATA_BLOCKS | ENABLE_PLUSES_DELIMITED_METADATA_BLOCKS | ENABLE_GFM`. Hover và preview cùng dùng.
  - `Block::List { start: Option<u64>, items: Vec<ListItem> }` (thay `ordered: bool`; `start.is_some()` là có thứ tự).
  - `pub struct ListItem { pub checked: Option<bool>, pub blocks: Vec<Block> }`.
  - `Block::Table { aligns: Vec<Align>, head: Vec<Vec<Inline>>, rows: Vec<Vec<Vec<Inline>>> }`, `pub enum Align { None, Left, Center, Right }`.
  - `Block::Image { url: String, alt: String }` khi đoạn chỉ gồm đúng một ảnh (bỏ khoảng trắng).
  - `Block::Alert { kind: AlertKind, blocks: Vec<Block> }` cho `> [!NOTE]` v.v. (`AlertKind`: `Note | Tip | Important | Warning | Caution`).
  - `Inline::Strike(Vec<Inline>)`, `Inline::Image { url: String, alt: String }`.
  - `MarkdownBlocks::root_lines(&self) -> &[Range<u32>]` (song song với `blocks()`, 0-based, cuối không tính), `MarkdownBlocks::line_count(&self) -> u32`, `MarkdownBlocks::code_index_start(&self, root: usize) -> usize` (chỉ số `code_runs` của code block đầu tiên trong khối gốc `root`).
  - `pub fn parse_markdown_document(source: &str, fallback_language: Option<&str>) -> MarkdownBlocks`: không cắt 100 KB (người gọi kiểm 5 MB). `parse_markdown` giữ chữ ký, cắt rồi gọi chung một hàm nội bộ.
- Front matter (metadata block) bị bỏ, không thành khối. Raw HTML vẫn bỏ.

- [ ] **Step 1: Viết test hỏng** trong `tests/markdown_parse.rs`:

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

- [ ] **Step 2: Chạy** `cargo test -p lucerna-workbench --test markdown_parse` → không biên dịch (kiểu chưa có).

- [ ] **Step 3: Cài đặt.** Dùng `Parser::new_ext(src, PARSE_OPTIONS).into_offset_iter()`; khi một khối gốc kết thúc (độ sâu frame về gốc), ghi `range.start..range.end` byte rồi đổi sang dòng qua bảng vị trí `\n` (dòng bắt đầu = dòng của byte đầu; dòng kết thúc = dòng của byte cuối `range.end - 1` cộng 1). `line_count` = số `\n` + 1 nếu file không kết thúc bằng `\n`, ngược lại = số `\n`. Tag `MetadataBlock` bị nuốt (không phát text). `TaskListMarker(b)` đặt `checked` cho mục đang mở. `BlockQuote(Some(kind))` → `Alert`. Tag `Image` trong đoạn gom thành `Inline::Image` (alt = văn bản con nối lại); đoạn chỉ có một `Inline::Image` → `Block::Image`. Renderer hiện tại: cập nhật nhánh `List` theo `ListItem`/`start`, vẽ `Strike` bằng `strikethrough`, vẽ `Alert` như `Quote` có tiêu đề đậm (`Note`, `Tip`, …); `Table` và `Image` tạm vẽ `div()` rỗng (Task 2 thay). `nth_code` duyệt cả `Alert`, `ListItem.blocks`, ô bảng không chứa code.

- [ ] **Step 4: Chạy** `cargo test -p lucerna-workbench --test markdown_parse --test hover` → PASS (hover cũ không đổi).

- [ ] **Step 5: Commit** `feat(markdown): tables, task lists, images and block lines in the parser`.

---

### Task 2: Renderer theo khối với `MarkdownStyle`

**Files:**
- Modify: `crates/lucerna-workbench/src/markdown.rs` (phần Rendering)
- Modify: `crates/lucerna-workbench/src/hover.rs` (chỉ nếu chữ ký `render_markdown_with` đổi — không nên đổi)
- Test: `crates/lucerna-workbench/tests/hover.rs` (thêm), `tests/markdown_render.rs` (mới)

**Interfaces:**
- Consumes: Task 1.
- Produces:
  - `pub struct MarkdownStyle { pub text_size: f32, pub line_height: f32 /* hệ số */, pub block_gap: f32, pub heading_sizes: [f32; 6], pub heading_rules: bool /* kẻ dưới h1,h2 */, pub code_scroll_x: bool, pub images: Option<ImageLookup> }`; `MarkdownStyle::hover()` tái tạo đúng giá trị hiện tại (13 px, gap 8, heading 18/16/15/14/14/14, không kẻ, không cuộn ngang, `images: None`); `MarkdownStyle::preview(font_size: f32, line_height: f32, images: Option<ImageLookup>) -> Self` với heading 2.0/1.5/1.25/1/1/1 × `font_size`, kẻ dưới h1/h2 bằng `border_variant`, khoảng trên heading 24 px, dưới 12 px.
  - `pub type ImageLookup = Rc<dyn Fn(&str /* url */, &str /* alt */, usize /* số thứ tự ảnh */, &App) -> AnyElement>`: renderer gọi nó cho mỗi ảnh khi có; Task 8 cung cấp.
  - `pub fn render_block(blocks: &MarkdownBlocks, root: usize, style: &MarkdownStyle, on_link: LinkHandler, cx: &App) -> AnyElement` — một khối gốc, đánh số code từ `code_index_start(root)`.
  - `render_markdown_with` giữ chữ ký, bên trong dùng `MarkdownStyle::hover()` và `render_block` cho từng khối.
- Kiểu khối theo spec §4.6: bảng viền 1 px `border_variant`, hàng đầu đậm nền `elevated_surface_background`, căn lề theo `Align`, cuộn ngang riêng bảng; task list dùng `ui::checkbox(checked, cx)` thay dấu đầu dòng (không có handler click); code block nền `elevated_surface_background` (hoặc màu hover đang dùng nếu khác — giữ một màu cho cả hai), bo `GRID`, padding 12 px, `code_scroll_x` thì `overflow_x_scroll` và không ngắt dòng; trích dẫn vạch trái 3 px `border`, chữ `text_muted`.
- Ảnh khi `style.images` là `None`: khung viền mờ, icon `icons::IMAGE`, chữ alt (`image_placeholder(alt, note: Option<&str>, cx) -> AnyElement`, `pub(crate)`, Task 8 dùng lại).

- [ ] **Step 1: Test hỏng.** Trong `tests/markdown_render.rs` (host giống `tests/hover.rs`): `table_renders_cells` — vẽ `"| a | b |\n|--|--|\n| 1 | 2 |\n"` bằng `render_block(.., 0, &MarkdownStyle::preview(14., 1.6, None), ..)`, rồi `cx.debug_bounds("md-table-cell-1-1")` là `Some` (gắn `debug_selector` cho ô `md-table-cell-{row}-{col}`, hàng đầu là row 0). `task_list_draws_checkboxes` — `"- [x] a\n- [ ] b\n"` có `md-task-0` và `md-task-1`. `image_without_context_shows_alt` — có `md-image-alt`. Trong `tests/hover.rs`: `hover_layout_unchanged` — chiều cao popover của `DOC` hiện có bằng giá trị đo trước khi sửa (ghi số đo ở Step 2 vào test).
- [ ] **Step 2: Chạy** `cargo test -p lucerna-workbench --test markdown_render --test hover` → FAIL (`render_block` chưa có). Trước khi sửa renderer, đo chiều cao popover hiện tại để điền vào `hover_layout_unchanged`.
- [ ] **Step 3: Cài đặt** như Interfaces.
- [ ] **Step 4: Chạy lại** → PASS.
- [ ] **Step 5: Commit** `feat(markdown): per-block renderer with hover and preview styles`.

---

### Task 3: Tab preview trong model nhóm editor

**Files:**
- Modify: `crates/lucerna-workbench/src/editor_groups/model.rs`
- Modify: `crates/lucerna-workbench/src/lib.rs` (re-export kiểu mới)
- Test: `crates/lucerna-workbench/tests/editor_groups_model.rs`

**Interfaces:**
- Produces:
  - `TabItem::MarkdownPreview(Arc<Path>)` (tĩnh) và `TabItem::MarkdownPreviewDynamic(Arc<Path>)` (file đang theo).
  - `TabItem::is_singleton`: `true` cho `MarkdownPreviewDynamic`, `false` cho `MarkdownPreview`. Singleton so theo loại: thêm `fn same_slot(&self, other: &TabItem) -> bool` (dynamic khớp mọi dynamic; còn lại `==`) và dùng nó trong `restore` và `open_singleton`.
  - `Tab::path()`: `Some` cho `File` và `MarkdownPreview`; `None` cho dynamic. `Tab::previewed_path() -> Option<&Arc<Path>>`: path của cả hai loại preview. `Tab::title()`: `MarkdownPreview` = tên file; dynamic = `format!("Preview {name}")`.
  - `pub enum FileView { Source, Preview }`.
  - `EditorGroups::open_file(&mut self, path: Arc<Path>, view: FileView, mode: OpenMode, preview_enabled: bool, reuse_any: bool) -> TabId`. `open(path, mode, pe)` = `open_file(path, FileView::Source, mode, pe, false)`. `reuse_any`: nhóm đích đã có `File(path)` hoặc `MarkdownPreview(path)` thì kích hoạt tab đó (tab active trước, rồi theo `mru`), không tạo mới; luật ghim/thay preview tab y như `open`.
  - `EditorGroups::replace_item(&mut self, id: TabId, item: TabItem) -> Option<TabId>`: thay tab `id` bằng tab mới cùng vị trí, cùng `pinned`, active nếu tab cũ active; nếu nhóm đã có tab với `item` thì đóng tab `id` và kích hoạt tab kia.
  - `EditorGroups::open_dynamic_preview(&mut self, path: Arc<Path>) -> TabId`: có dynamic ở đâu thì đổi path và kích hoạt nó (nhóm của nó thành active? **không**: giữ nhóm active hiện tại, chỉ đặt tab active trong nhóm của nó); chưa có thì mở ghim ở nhóm bên phải nhóm active (tạo nếu cần, như `ToSide`) rồi **trả nhóm active về nhóm cũ** (focus ở lại editor).
  - `EditorGroups::retarget_dynamic(&mut self, path: Arc<Path>) -> bool`: đổi path của dynamic nếu có và khác.
  - `EditorGroups::dynamic_preview(&self) -> Option<(usize, &Tab)>`.
  - Lưu phiên: `pub const MARKDOWN_PREVIEW_STATE_PREFIX: &str = "lucerna:markdown-preview:"`, `pub const MARKDOWN_PREVIEW_DYNAMIC_STATE_PREFIX: &str = "lucerna:markdown-preview-dynamic:"`; `saved_path`/`saved_item` ghép/tách tiền tố với đường dẫn tuyệt đối.

- [ ] **Step 1: Test hỏng** (thêm vào `tests/editor_groups_model.rs`; helper `tabs()` đổi sang dùng `title()` hoặc `previewed_path()` cho tab preview):
  - `plain_open_reuses_source_tab`: `open(a.md, Pinned)` rồi `open_file(a.md, Preview, Preview, true, true)` → một tab, `TabItem::File`, cùng id.
  - `plain_open_reuses_static_preview`: ngược lại, preview có sẵn được kích hoạt.
  - `point_open_adds_source_next_to_preview`: có `MarkdownPreview(a.md)`, `open(a.md, Pinned)` → hai tab, preview vẫn còn.
  - `replace_item_keeps_slot_and_pin`: ba tab, thay tab giữa (preview tab, chưa ghim) → cùng vị trí, `pinned == false`, active.
  - `replace_item_into_existing_closes_old`.
  - `dynamic_preview_is_singleton_and_keeps_focus`: hai lần `open_dynamic_preview(a.md)`, `(b.md)` → một tab dynamic ở nhóm 1, title `"Preview b.md"`, `active_group() == 0`.
  - `previews_saved_and_restored`: `state()` chứa `"lucerna:markdown-preview:/x/a.md"` và `"lucerna:markdown-preview-dynamic:/x/b.md"`; `restore` cho lại đúng `TabItem`; hai dynamic trong state chỉ giữ cái đầu.
  - `static_preview_has_path_dynamic_has_not`.
- [ ] **Step 2: Chạy** `cargo test -p lucerna-workbench --test editor_groups_model` → FAIL.
- [ ] **Step 3: Cài đặt** như Interfaces. Sửa mọi `match` trên `TabItem` trong crate để biên dịch: `workbench.rs` tab body factory (tạm `MarkdownPreview*` dùng nhánh `File` của path cho tới Task 4), `editor_groups/view.rs` icon (`None` tạm thời).
- [ ] **Step 4: Chạy** `cargo test -p lucerna-workbench --test editor_groups_model --test editor_groups_view --test workspace_state` → PASS.
- [ ] **Step 5: Commit** `feat(editor-groups): static and dynamic markdown preview tabs`.

---

### Task 4: View `MarkdownPreview` — vẽ, giữ document, tab của file

**Files:**
- Create: `crates/lucerna-workbench/src/markdown_preview/mod.rs` (view), `crates/lucerna-workbench/src/markdown_preview/sync.rs` (để trống tới Task 6 nếu chưa cần)
- Modify: `crates/lucerna-workbench/src/lib.rs`, `src/workbench.rs` (tab body factory), `src/editor_groups/view.rs` (icon tab `icons::EYE`, `disk_bar` cho preview tĩnh), `src/icons.rs` + `crates/lucerna-workbench/assets/icons/eye.svg` (Lucide `eye`, ISC, như các icon khác), `crates/lucerna-settings/assets/default-settings.json`
- Test: `crates/lucerna-workbench/tests/markdown_preview.rs` (mới)

**Interfaces:**
- Consumes: Task 1–3.
- Produces:
  - `pub struct MarkdownPreview` (entity) với `MarkdownPreview::new(path: Arc<Path>, dynamic: bool, documents: Entity<Documents>, workbench: WeakEntity<Workbench>, cx: &mut Context<Self>) -> Self`.
  - `pub fn path(&self) -> &Arc<Path>`, `pub fn set_path(&mut self, path: Arc<Path>, cx)` (dynamic đổi file: thả `CodeView` passive cũ, mở cái mới, parse ngay).
  - `pub fn blocks(&self) -> Option<&Arc<MarkdownBlocks>>`, `pub fn list_state(&self) -> &ListState`.
  - `pub fn top_line(&self) -> f32` / `pub fn scroll_to_line(&mut self, line: f32, cx)` — khai báo ở Task 4 với cài đặt đơn giản (về khối chứa dòng, offset 0); Task 6 dùng ánh xạ đầy đủ.
  - Giữ document: `documents.open_preview(path)` → `Entity<CodeView>` passive, giữ trong struct, không vẽ (spec §5.3 cho phép).
  - Trạng thái: chưa có snapshot → div trống; `DocState::Deleted`/lỗi → `disk_bar` của nhóm (giống tab mã nguồn, view.rs tra bằng `Tab::path()`); `len_bytes > MAX_PREVIEW_BYTES` → thông báo "File quá lớn để xem trước" + `text_button("Show Source")` chạy `markdown.showSource`.
  - Render: `list(list_state, |ix| render_block(..ix..))` trong khung căn giữa rộng tối đa 800 px, padding 32/24, nền `editor_background`; `MarkdownStyle::preview(fontSize, lineHeight, ..)` đọc `markdown.preview.fontSize` (14), `markdown.preview.lineHeight` (1.6), `markdown.preview.fontFamily` ("" = `UI_FONT_FAMILY`).
  - Settings mặc định thêm vào `default-settings.json`: `"markdown.preview.fontSize": 14`, `"markdown.preview.lineHeight": 1.6`, `"markdown.preview.fontFamily": ""`.
  - Tab body factory: `MarkdownPreview(path)` → view mới (`dynamic: false`); `MarkdownPreviewDynamic(path)` → view mới (`dynamic: true`). Tab dynamic không có dirty dot (vì `path()` là `None`).

- [ ] **Step 1: Test hỏng** trong `tests/markdown_preview.rs` (helper `init`/`open`/`draw` như `tests/open_files.rs`; thêm `open_preview(wb, path, cx) -> Entity<MarkdownPreview>` mở `TabItem::MarkdownPreview` qua model rồi downcast `active_body`):
  - `preview_renders_blocks`: file 3 khối → `list_state().item_count() == 3`, `cx.debug_bounds("md-table-cell-0-0")` có.
  - `closing_dirty_source_keeps_preview_quiet`: mở tab mã nguồn, gõ (dirty), mở preview tĩnh cùng nhóm, đóng tab mã nguồn → không có prompt (dùng cách `tests/save.rs` kiểm prompt), preview vẫn có khối chứa chữ vừa gõ; tab preview có dirty dot (`debug_selector` dot hiện có).
  - `too_large_file_shows_notice`: file 5 MB + 1 byte → `debug_bounds("md-preview-too-large")` có, `blocks()` là `None`.
  - `deleted_file_shows_disk_bar`.
- [ ] **Step 2: Chạy** → FAIL.
- [ ] **Step 3: Cài đặt** (parse lần đầu có thể đồng bộ trong `new` ở task này; Task 5 chuyển sang nền).
- [ ] **Step 4: Chạy** `cargo test -p lucerna-workbench --test markdown_preview --test save --test editor_groups_view` → PASS.
- [ ] **Step 5: Commit** `feat(markdown-preview): preview view as a tab of its file`.

---

### Task 5: Cập nhật nền có debounce, giữ chỗ đang đọc

**Files:**
- Modify: `crates/lucerna-workbench/src/markdown_preview/mod.rs`
- Test: `crates/lucerna-workbench/tests/markdown_preview.rs`

**Interfaces:**
- Consumes: `DocumentEvent::{Opened, Edited}` của `Documents` (lọc đúng `path`), `Documents::snapshot_of`, `revision_of`.
- Produces: `pub const REPARSE_DELAY: Duration = Duration::from_millis(200)`; `pub fn parses_started(&self) -> u64` (đếm, cho test); `pub fn refresh(&mut self, cx)` (bỏ cache ảnh, parse lại ngay — Task 7 gán lệnh).
- Luật: lần đầu parse ngay; sự kiện tới khi chưa có lần chờ thì hẹn `REPARSE_DELAY` rồi lấy snapshot **lúc đó**; đã có lần chờ thì bỏ qua. Parse bằng `cx.background_spawn` trên chuỗi của snapshot; kết quả có `revision` cũ hơn kết quả đang hiện thì bỏ. Trước khi thay: `top = top_line()`; sau `list_state.reset(n)`: `scroll_to_line(top)`. Đổi theme/font chỉ `notify`.

- [ ] **Step 1: Test hỏng:**
  - `rewrite_on_disk_updates_preview`: mở preview, `fs::write` nội dung mới, gửi `WorktreeEvent::Changed(Modified)` như `tests/external_changes.rs:153`, `cx.executor().advance_clock(REPARSE_DELAY)`, `draw` → khối mới hiện.
  - `burst_of_writes_parses_last_once`: 5 lần ghi + sự kiện trong 100 ms (advance 20 ms mỗi lần) → sau `advance_clock(REPARSE_DELAY)`: `parses_started()` tăng đúng 1 và nội dung là lần ghi thứ 5.
  - `rewrite_keeps_reading_position`: file 200 heading (`# h{i}\n\npara\n\n`), cuộn tới khối 120, ghi lại file với 1 heading mới chèn ở đầu → khối trên cùng sau cập nhật là khối mang `# h119` hoặc `# h120` (theo dòng), không phải 0.
  - `typing_updates_without_save`: gõ vào tab mã nguồn ở nhóm khác → preview đổi sau debounce.
- [ ] **Step 2: Chạy** → FAIL.
- [ ] **Step 3: Cài đặt.**
- [ ] **Step 4: Chạy** → PASS.
- [ ] **Step 5: Commit** `feat(markdown-preview): background reparse with debounce, keep position`.

---

### Task 6: Ánh xạ dòng ↔ cuộn và cuộn đồng bộ

**Files:**
- Modify: `crates/lucerna-workbench/src/markdown_preview/sync.rs`, `markdown_preview/mod.rs`
- Modify: `crates/lucerna-workbench/src/code_view/mod.rs` (sự kiện cuộn)
- Modify: `src/workbench.rs` (nối editor ↔ preview cùng file), `default-settings.json`
- Test: `crates/lucerna-workbench/tests/markdown_sync.rs` (mới, thuần), `tests/markdown_preview.rs`

**Interfaces:**
- Produces (thuần, `pub`):
  - `pub fn line_to_item(root_lines: &[Range<u32>], line_count: u32, line: f32) -> (usize, f32)` — khối cuối có `start ≤ line`; `fraction = (line − start) / max(1, next_start − start)` với `next_start` là dòng đầu khối kế (hoặc `line_count`), kẹp `[0, 1)`; trước khối đầu → `(0, 0.)`; danh sách rỗng → `(0, 0.)`.
  - `pub fn item_to_line(root_lines: &[Range<u32>], line_count: u32, item: usize, fraction: f32) -> f32` — nghịch đảo.
  - `CodeView` phát `pub enum CodeViewEvent { ScrolledTo(u32) }` khi `scroll_top_row` đổi do người dùng hoặc `set_scroll_top_row` (`impl EventEmitter<CodeViewEvent> for CodeView`), dòng là dòng buffer (đã bỏ qua fold).
  - `MarkdownPreview::top_line` / `scroll_to_line` dùng hai hàm trên với `list_state.logical_scroll_top()` và `bounds_for_item(ix)` (chưa đo → offset 0, sửa ở frame sau qua một cờ chờ).
  - Settings mặc định: `"markdown.preview.scrollPreviewWithEditor": true`, `"markdown.preview.scrollEditorWithPreview": true`.
  - Nối trong `Workbench`: với mỗi `MarkdownPreview` đang hiện và mỗi `CodeView` đang hiện của cùng `path` ở **nhóm khác**: editor `ScrolledTo(row)` → `preview.scroll_to_line(row as f32)` nếu setting bật; `list_state.set_scroll_handler` (chỉ chạy khi người dùng cuộn) → `view.set_scroll_top_row(top_line().floor() as u32)` nếu setting bật, đặt cờ `syncing` để bỏ đúng một `ScrolledTo` phản hồi.
  - Đổi tại chỗ (Task 7 dùng): `pub(crate) fn source_line_for_preview(preview) -> u32` và mở preview với dòng ban đầu.
- [ ] **Step 1: Test hỏng** trong `tests/markdown_sync.rs`: `before_first_block`, `inside_block_fraction` (`[0..2, 4..10]`, dòng 7 → `(1, 0.5)`), `blank_line_belongs_to_previous` (dòng 3 → `(0, 0.75)`), `after_last_block` (dòng 20, `line_count` 20 → `(1, ~0.999)`), `empty_doc`, `roundtrip` (với 50 dòng ngẫu nhiên cố định: `item_to_line(line_to_item(l)) ≈ l` sai số < 1e-3). Trong `tests/markdown_preview.rs`: `editor_scroll_moves_side_preview` (nhóm 0 mã nguồn, nhóm 1 preview; `set_scroll_top_row(150)` → khối trên cùng của preview chứa dòng 150), `setting_off_stops_sync`.
- [ ] **Step 2: Chạy** `cargo test -p lucerna-workbench --test markdown_sync --test markdown_preview` → FAIL.
- [ ] **Step 3: Cài đặt.**
- [ ] **Step 4: Chạy** → PASS; thêm `cargo test -p lucerna-workbench --test code_view` vẫn PASS.
- [ ] **Step 5: Commit** `feat(markdown-preview): two-way scroll sync with the editor`.

---

### Task 7: Lệnh, phím, nút, association, context key

**Files:**
- Create: `crates/lucerna-workbench/src/markdown_preview/commands.rs`
- Modify: `src/workbench.rs` (`open_path`, mới `open_source`, `open_path_at`, `navigate`, `context_keys`, đăng ký lệnh ở ~1851), `src/editor_groups/view.rs` (nút ở mép phải path bar), `crates/lucerna-syntax/src/associations.rs` (`matches` thành `pub`, doc chung cho mọi bảng glob), `crates/lucerna-settings/assets/default-keybindings.json`, `default-settings.json`, `src/app_menu.rs` nếu menu View liệt kê lệnh (thêm "Open Preview to the Side")
- Test: `tests/markdown_preview.rs`, `tests/open_files.rs`, `crates/lucerna-settings/tests/keymap.rs`

**Interfaces:**
- Produces:
  - `Workbench::open_source(&mut self, path: Arc<Path>, mode: OpenMode, cx)` — nội dung cũ của `open_path`. `open_path_at` và `navigate` gọi `open_source`.
  - `Workbench::open_path(path, mode, cx)` — tra `workbench.editorAssociations` (`BTreeMap<String, String>`, đường dẫn tương đối gốc như `files.associations`) qua `Associations::new(&map).matches(rel)`: giá trị đầu là `"vscode.markdown.preview.editor"` → `open_file(path, FileView::Preview, mode, pe, true)`; `"default"` hoặc không khớp → `open_file(path, FileView::Source, mode, pe, true)`. Ghi recent như cũ. Thêm `pub fn editor_for(path: &Path, rel: &Path, map: &BTreeMap<String,String>) -> FileView` thuần để test.
  - Mặc định: `"workbench.editorAssociations": { "*.md": "vscode.markdown.preview.editor", "*.markdown": "vscode.markdown.preview.editor" }`, `"markdown.preview.doubleClickToSwitchToEditor": true`.
  - Lệnh (`commands.rs`, `register_commands(&mut CommandRegistry)`): `markdown.togglePreview` "Markdown: Toggle Preview", `markdown.showPreview` "Markdown: Open Preview", `markdown.showPreviewToSide` "Markdown: Open Preview to the Side", `markdown.showSource` "Markdown: Show Source", `markdown.preview.refresh` "Markdown: Refresh Preview". Hành vi đúng bảng spec §4.2. Đổi tại chỗ dùng `replace_item` và giữ vị trí: mã nguồn → preview: `scroll_to_line(view.scroll_top_row())`; preview → mã nguồn: `go_to_and_scroll_when_opened(path, Point{row: line,0}, line)` với `line = top_line().floor()`. Từ dynamic, `togglePreview`/`showSource` mở mã nguồn ở nhóm active (`open_source`, Pinned). Lệnh chạy khi tab active không phải markdown thì không làm gì.
  - Double-click khối (setting bật): preview tĩnh → `showSource` tại `root_lines[ix].start`; dynamic → nhóm có `CodeView` của file thì kích hoạt nó và `go_to`, không có thì `open_source` ở nhóm của preview.
  - Dynamic theo tab active: khi tab active của một nhóm khác nhóm chứa dynamic là `File` hoặc `MarkdownPreview` của một file có ngôn ngữ `markdown` (`languages.language_for_file(rel, "")`), `retarget_dynamic(path)` và `set_path`, rồi `scroll_to_line` theo editor nếu có.
  - Context keys: `activeEditor` = `"markdownPreview"` khi body active là `MarkdownPreview`; `editorLangId` và `resourceExtname` của preview dynamic lấy từ `MarkdownPreview::path()` (preview tĩnh đã có qua `Tab::path()`).
  - Phím (`default-keybindings.json`): `{ "key": "cmd+shift+v", "command": "markdown.togglePreview", "when": "editorLangId == markdown || activeEditor == 'markdownPreview'" }`, `{ "key": "cmd+k v", "command": "markdown.showPreviewToSide", "when": "editorLangId == markdown" }`. Kiểm `cmd+shift+v` chưa bị gán cho lệnh khác trong file; nếu có, ghi lại và để binding markdown sau (thắng theo thứ tự VS Code).
  - Nút path bar: `icon_button("md-preview-toggle-{gi}", icons::EYE, false, cx)` tooltip "Open Preview to the Side (⌘K V)", click → `markdown.showPreviewToSide`, Alt+click → `markdown.togglePreview`; trên preview: `icons::FILE_CODE`, tooltip "Show Source (⌘⇧V)", click → `markdown.togglePreview`. Chỉ hiện khi tab active của nhóm là markdown. Phím trong tooltip lấy từ keymap thật (như các tooltip khác), không viết cứng.
  - Phím cuộn trong preview: ↑/↓ (một dòng chữ = `fontSize × lineHeight`), PageUp/Down, Space/Shift+Space, Home/End, ⌘↑/⌘↓ — xử lý trong `on_key_down` của view, không thêm lệnh.
- [ ] **Step 1: Test hỏng:**
  - `tests/open_files.rs`: `md_opens_as_preview_by_default` (Explorer-style `open_path(a.md, Preview)` → tab `MarkdownPreview`), `md_jump_opens_source` (`open_path_at(a.md, .., Point{row:10})` → `File`), `go_back_returns_to_source` (nhảy hai lần, `navigate(back)` → `File`), `association_default_opens_source` (user settings `{"workbench.editorAssociations": {"*.md": "default"}}`), `association_full_path_glob` (`"docs/**/*.md"` khớp; `README.md` ở gốc thì không), `rs_unaffected`.
  - `tests/markdown_preview.rs`: `toggle_swaps_in_place_and_back` (cùng vị trí tab, cùng pinned, dòng trên cùng ±1 khối), `show_to_side_keeps_focus_in_editor` (`active_group()` vẫn 0), `dynamic_follows_active_markdown` (mở b.md ở nhóm 0 → dynamic là `Preview b.md`; mở `x.rs` → vẫn b.md), `double_click_opens_source_at_block`, `context_keys_in_preview` (`editorLangId == "markdown"`, `activeEditor == "markdownPreview"` cho cả tĩnh và dynamic), `cmd_shift_v_dispatches_toggle` (qua keymap thật).
  - `crates/lucerna-settings/tests/keymap.rs`: `markdown_bindings_parse` (không lỗi `when`).
- [ ] **Step 2: Chạy** → FAIL.
- [ ] **Step 3: Cài đặt.**
- [ ] **Step 4: Chạy** `cargo test -p lucerna-workbench --test markdown_preview --test open_files --test nav_history --test workspace_state` và `cargo test -p lucerna-settings -p lucerna-syntax` → PASS.
- [ ] **Step 5: Commit** `feat(markdown-preview): commands, keys, path bar button and editor associations`.

---

### Task 8: Ảnh local và link

**Files:**
- Create: `crates/lucerna-workbench/src/markdown_preview/resolve.rs`
- Modify: `src/markdown.rs` (vẽ ảnh qua `img()` khi có `ImageContext`; `LinkHandler` của preview), `markdown_preview/mod.rs` (`ImageContext`, cache ảnh, cuộn tới slug), `default-settings.json`
- Test: `crates/lucerna-workbench/tests/markdown_resolve.rs` (mới, thuần), `tests/markdown_preview.rs`

**Interfaces:**
- Produces (thuần):
  - `pub enum ImageSrc { Local(PathBuf), Blocked(&'static str) }`; `pub fn resolve_image(url: &str, ctx: &ImageContext) -> ImageSrc`: `http(s):`/`data:` → `Blocked("remote")`; `/x` theo `ctx.root`; tương đối theo `ctx.base_dir`; decode `%xx`; chuẩn hoá `..` theo từ vựng; rồi `canonicalize` (lỗi → `Blocked("missing")`); không nằm dưới một phần tử của `ctx.allowed` (đã canonicalize) → `Blocked("outside")`; `metadata.len() > MAX_IMAGE_BYTES` → `Blocked("too-large")`. Hàm này chạm đĩa: job parse nền của Task 5, sau `parse_markdown_document`, gọi nó cho mọi URL ảnh và trả về `HashMap<String, ImageSrc>` cùng với khối; view giữ bảng đó và đưa vào `MarkdownStyle::preview(.., Some(lookup))` qua một `ImageLookup` đọc bảng. Không gọi khi vẽ.
  - `pub struct ImageContext { pub base_dir: PathBuf, pub root: PathBuf, pub allowed: Vec<PathBuf> }`.
  - `ctx.allowed` = `[root]` khi file nằm trong root; `[thư mục của file]` khi không (chế độ một file: root là thư mục của file).
  - `pub enum LinkAction { Web(String), Anchor(String), Markdown { path: PathBuf, slug: Option<String> }, Source { path: PathBuf, point: Point }, File(PathBuf), None }`; `pub fn resolve_link(url: &str, base_dir: &Path, root: &Path) -> LinkAction`: `http(s):`/`mailto:` → `Web`; `#slug` → `Anchor`; đường dẫn tương đối, `/x` hoặc `file://` tới `.md`/`.markdown` → `#L…` thì `Source` (dùng `line_fragment` hiện có), không thì `Markdown`; file khác → `File` (có `#L…` thì `Source`).
  - `pub fn heading_slugs(blocks: &MarkdownBlocks) -> Vec<(String, usize)>` (slug kiểu GitHub: chữ thường, bỏ ký tự không phải chữ/số/khoảng trắng/`-`, khoảng trắng → `-`, trùng thêm `-1`, `-2`; giá trị là chỉ số khối gốc).
  - Thực thi link trong preview: `Web` → `cx.open_url`; `Anchor` → cuộn tới khối; `Markdown` → `markdown.preview.openMarkdownLinks` (`"inPreview"` mặc định, thêm vào `default-settings.json`) ⇒ preview tĩnh: `open_path(path, Preview)`, dynamic: `retarget_dynamic` + `set_path`; `"inEditor"` ⇒ `open_source`; `slug` thì cuộn sau khi parse; `Source` → `jump_to`; `File` → `open_path(.., Preview)`. Hover giữ `open_markdown_link` cũ.
  - Ảnh: `img(path).max_w_full()` giữ tỉ lệ, không phóng to quá cỡ thật, cache theo view (`RetainAllImageCache` gắn vào `MarkdownPreview`, thả khi view drop; `refresh` xoá nó). `Blocked(reason)` → `image_placeholder(alt, note)` với note `"Ảnh quá lớn"` cho `too-large`, không note cho các lý do khác. Tooltip của link là URL.
- [ ] **Step 1: Test hỏng** `tests/markdown_resolve.rs` (dùng `TempDir`): `relative_image`, `root_absolute_image`, `percent_decoded`, `image_outside_root_is_blocked` (cả `../../x.png` lẫn symlink trong root trỏ ra ngoài), `missing_image`, `remote_image_blocked`, `too_large_image` (file thưa 20 MB + 1 qua `set_len`), `loose_file_allows_its_dir`; `link_kinds` (bảng các URL → `LinkAction`), `slugs_github_style` (`"Hello, World!"` → `hello-world`; hai `"A"` → `a`, `a-1`; tiếng Việt `"Tổng quan"` → `tổng-quan`). Trong `tests/markdown_preview.rs`: `local_image_drawn` (`debug_bounds("md-image-0")`), `anchor_link_scrolls`, `md_link_opens_preview_tab`.
- [ ] **Step 2: Chạy** → FAIL.
- [ ] **Step 3: Cài đặt.**
- [ ] **Step 4: Chạy** → PASS.
- [ ] **Step 5: Commit** `feat(markdown-preview): local images and links`.

---

### Task 9: "Open in Browser" cho HTML

**Files:**
- Modify: `src/open_commands.rs` (đăng ký), `src/quick_open/` nếu palette lọc lệnh theo điều kiện
- Test: `tests/open_commands.rs`

**Interfaces:**
- Produces: lệnh `workbench.action.openInBrowser`, tiêu đề "Open in Browser"; không gán phím. Chạy: tab active có `path()` đuôi `html`/`htm` (không phân biệt hoa thường) → `cx.open_url(url::Url::from_file_path(path))` (bản trên đĩa); khác → không làm gì. Palette chỉ liệt kê lệnh khi điều kiện đó đúng: nếu palette chưa có cơ chế ẩn lệnh theo ngữ cảnh, thêm vào `CommandRegistry` một `fn register_when(id, title, when: &'static str, handler)` đánh giá `when` bằng `context_keys` (`resourceExtname == .html || resourceExtname == .htm`) và palette bỏ lệnh có `when` sai.
- [ ] **Step 1: Test hỏng:** `open_in_browser_listed_only_for_html` (palette có lệnh khi tab là `a.html`, không có khi `a.rs`), `open_in_browser_url_is_file_url` (tách hàm thuần `browser_url(path: &Path) -> Option<String>`; `/tmp/a b.html` → `file:///tmp/a%20b.html`).
- [ ] **Step 2: Chạy** → FAIL. **Step 3: Cài đặt.** **Step 4: Chạy** → PASS.
- [ ] **Step 5: Commit** `feat: Open in Browser for HTML files`.

---

### Task 10: Đo binary, RAM, hiệu năng và review giao diện với Zed

**Files:**
- Modify: `docs/superpowers/specs/2026-11-13-markdown-preview-design.md` (thêm §10 "Số đo"), `tests/perf.rs` (thêm bench parse)
- Modify: `docs/superpowers/specs/2026-11-08-lucerna-phase1-design.md` §1.3 (thêm một dòng "Markdown preview (xong)")

- [ ] **Step 1: Mốc.** Trên commit ngay trước Task 1 (`git merge-base HEAD main` hoặc commit spec `d3c4043`): `git worktree add` tạm, `scripts/bundle-app.sh`, ghi `stat -f %z .../luc`; xoá worktree tạm. Chú ý chỉ giữ một profile trên đĩa.
- [ ] **Step 2: Sau.** `scripts/bundle-app.sh` trên HEAD, ghi kích thước. Chênh lệch ≤ 300 KB; nếu > 500 KB thì dừng và tìm nguyên nhân (feature `image` mới, generic phình) trước khi merge.
- [ ] **Step 3: Crate.** `git diff <mốc> -- Cargo.lock` rỗng.
- [ ] **Step 4: RAM.** Ba ca A/B/C của spec §6 bằng `footprint <pid>` sau 10 s; B − A ≤ 10 MB; C trở về gần A sau khi đóng tab.
- [ ] **Step 5: Parse.** Thêm `parse_5000_lines` vào `tests/perf.rs` (5000 dòng ~200 KB trộn heading/đoạn/bảng/code) — `parse_markdown_document` < 30 ms ở release; chạy theo cách `perf.rs` đang chạy.
- [ ] **Step 6: Giao diện.** Mở `docs/superpowers/specs/2026-11-08-lucerna-phase1-design.md` ở Lucerna (preview) và Zed (`markdown::OpenPreview`) cạnh nhau; so khoảng cách, cỡ chữ heading, bảng, code block, trích dẫn, trạng thái trống; sửa lệch không có lý do. Chụp màn hình vào ghi chú merge.
- [ ] **Step 7: Ghi số đo** vào spec §10 và commit `docs: markdown preview measurements`.

---

## Thứ tự và phụ thuộc

1 → 2 → (3 song song được với 1–2) → 4 → 5 → 6 → 7 → 8 → 9 (độc lập, làm lúc nào cũng được) → 10.
