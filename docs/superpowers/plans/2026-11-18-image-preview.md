# Image Preview Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Image files (PNG, JPEG, GIF, WebP, BMP, TIFF, ICO, SVG) open in their own tab that shows the image, its size in the status bar, and reloads when the file changes on disk.

**Architecture:** A new tab kind `TabItem::ImagePreview(path)`, chosen by `FileView::Image` in `editor_for` (VS Code's `imagePreview.previewEditor`, builtin priority). The body is a new `ImagePreview` entity in `crates/lucerna-workbench/src/image_preview/`. It reads and decodes the file on the background executor into an `Arc<RenderImage>` and draws it with `img(ImageSource::Render(..))`. It never touches `Documents`. SVG is rasterised by GPUI's `SvgRenderer::render_single_frame` at the window's scale factor.

**Tech Stack:** Rust, GPUI (pinned rev `b5cd310`), `image` 0.25 and `usvg` 0.46. Both crates are already in `Cargo.lock` through GPUI; they become direct deps of `lucerna-workbench`.

**Spec:** `docs/superpowers/specs/2026-11-18-image-preview-design.md` (research in `docs/research/2026-11-18-image-preview.md`)

## Global Constraints

- Before any `cargo test` or build:
  - `export PATH="/opt/homebrew/opt/rustup/bin:$PATH"`.
  - Run `scripts/prune-target.sh`.
  - Keep one build profile: delete `target/release` before a debug build or test, but never while another cargo runs (`pgrep -x cargo`).
  - Never set `CARGO_TARGET_DIR`.
  - Keep free disk space above about 1.2 GB.
- No new crate in `Cargo.lock`. `image` and `usvg` must resolve to the versions already locked (0.25.10 and 0.46.0). `cargo deny check` must pass. The release binary may grow by at most 200 KB.
- Only `lucerna-workbench` uses gpui. Every user-visible string is English.
- No change to `Documents`' load, save or hot-exit paths, except that "Open as Text" calls the existing `Documents::open_as_text`.
- Image extensions, compared case-insensitively: `png jpg jpeg jpe gif webp bmp ico tif tiff svg`.
- Editor id: `imagePreview.previewEditor`. Saved-tab prefix: `lucerna:image-preview:`.
- Limits: file larger than `MAX_IMAGE_BYTES` (20 MiB, reuse the markdown constant) → not read. Raster image above `MAX_IMAGE_PIXELS = 100_000_000` (width × height) → not decoded. An SVG over that limit is rasterised at a lowered scale instead.
- Reload debounce: 100 ms.
- Exact UI copy:
  - `The image is too large to preview (34.2 MB).`, with the size from `format_size`.
  - `The image is too large to preview (20000 × 12000).`
  - `The image could not be loaded.`
  - `This image is stored with Git LFS and has not been downloaded.`
  - Button `Open as Text`.
  - Commands `Reopen as Text` (`imagePreview.reopenAsText`) and `Reopen as Image Preview` (`imagePreview.reopenAsPreview`).
- Status text: `W × H • size`, with `×` U+00D7 and `•` U+2022, spaces around both. Size is base 1024: `512 B`, `3.2 KB`, `34.2 MB`, one decimal for KB and MB.
- Use shared `ui.rs` components and theme tokens. No hard-coded colours.
- Commits: stage by explicit path. End each message with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. **An agent rewrites a PNG many times in a row.** Expect one reload after the burst, no flash to empty, and RAM flat across 50 rewrites. Pinned by `rewrites_in_a_burst_reload_once` (Task 4).
2. **The file is truncated or half-written at reload time.** Expect the old image to stay and no error screen. Pinned by `failed_reload_keeps_old_image` (Task 4).
3. **`.PNG` / `.SVG` written in upper case,** which is common in screenshots from Windows. Expect them to open as images. Pinned by `editor_for_image_extensions_ignore_case` (Task 1).
4. **A file named `.png` but not a PNG** (empty, text, or a renamed `.webp`). An empty or text file shows `The image could not be loaded.` with `Open as Text`. A misnamed but valid image still shows, because the format is guessed from the bytes. Pinned by `wrong_extension_still_decodes` and `empty_file_shows_error` (Task 2).
5. **The tab is closed while decoding is still running.** Expect no panic, and the late result is dropped. Pinned by `close_during_decode_is_harmless` (Task 2).

---

### Task 1: Pure helpers, `FileView::Image` and the image tab in the model

**Files:**
- Create: `crates/lucerna-workbench/src/image_preview/mod.rs` (module root; this task adds only `pub mod info;` and re-exports)
- Create: `crates/lucerna-workbench/src/image_preview/info.rs`
- Modify: `crates/lucerna-workbench/src/editor_groups/model.rs`:
  - `TabItem` (~l.27): add the variant.
  - `is_singleton` (~l.70): exclude the new variant.
  - `FileView` (~l.85): add `Image`.
  - `path()` and title (~l.147–170).
  - `open_file` (~l.305–310): map `FileView::Image`, and extend `shows_file`.
  - `retarget` (~l.570).
  - `saved_item`, `split_preview_state`, `saved_path` (~l.1000–1060).
- Modify: `crates/lucerna-workbench/src/editor_groups/closed.rs:43,51,109`: treat `ImagePreview` like `MarkdownPreview`.
- Modify: `crates/lucerna-workbench/src/markdown_preview/commands.rs:119-133`: `editor_for`.
- Modify: `crates/lucerna-workbench/src/lib.rs`: add `mod image_preview;` and the re-exports below.
- Test: `crates/lucerna-workbench/tests/image_preview_info.rs` (new), `tests/open_files.rs`, `tests/editor_groups_model.rs`

**Interfaces:**
- Produces, from `image_preview::info`, re-exported from the crate root:
  - `pub const IMAGE_EDITOR_ID: &str = "imagePreview.previewEditor";`
  - `pub const IMAGE_EXTENSIONS: &[&str]`
  - `pub const MAX_IMAGE_PIXELS: u64 = 100_000_000;`
  - `pub fn is_image_path(path: &Path) -> bool`
  - `pub fn is_svg_path(path: &Path) -> bool`
  - `pub fn format_size(bytes: u64) -> String`
  - `pub fn status_text(dims: Option<(u32, u32)>, bytes: u64) -> String`: `"512 × 512 • 3.2 KB"`, or just `"3.2 KB"` when `dims` is `None`.
  - `pub fn is_lfs_pointer(bytes: &[u8]) -> bool`: true when the bytes start with `version https://git-lfs.github.com/spec/`.
- Produces, in the model:
  - `TabItem::ImagePreview(Arc<Path>)`
  - `FileView::Image`
  - `pub const IMAGE_PREVIEW_STATE_PREFIX: &str = "lucerna:image-preview:";`
  - `Tab::path()` returns `Some(path)` for `ImagePreview`.
  - `Tab::previewed_path()` stays markdown-only.

- [ ] **Step 1: Write the failing tests**

In `tests/image_preview_info.rs`:
```rust
#[test] fn format_size_units() {
    assert_eq!(format_size(512), "512 B");
    assert_eq!(format_size(3277), "3.2 KB");
    assert_eq!(format_size(34 * 1024 * 1024 + 200 * 1024), "34.2 MB");
}
#[test] fn status_text_joins_dims_and_size() {
    assert_eq!(status_text(Some((512, 512)), 3277), "512 \u{d7} 512 \u{2022} 3.2 KB");
    assert_eq!(status_text(None, 512), "512 B");
}
#[test] fn image_paths() {
    assert!(is_image_path(Path::new("/a/logo.PNG")));
    assert!(is_image_path(Path::new("x.jpe")));
    assert!(!is_image_path(Path::new("x.avif")));
    assert!(is_svg_path(Path::new("i.SVG")));
}
#[test] fn lfs_pointer() {
    assert!(is_lfs_pointer(b"version https://git-lfs.github.com/spec/v1\noid sha256:ab\n"));
    assert!(!is_lfs_pointer(b"\x89PNG"));
}
```

In `tests/open_files.rs`, next to `editor_for_matches_relative_and_absolute_globs`:
```rust
#[test] fn editor_for_image_extensions_ignore_case() {
    let m = map(&[]);
    assert_eq!(editor_for(Path::new("/r/a.PNG"), Path::new("a.PNG"), &m), FileView::Image);
    assert_eq!(editor_for(Path::new("/r/i.svg"), Path::new("i.svg"), &m), FileView::Image);
    assert_eq!(editor_for(Path::new("/r/a.rs"), Path::new("a.rs"), &m), FileView::Source);
}
#[test] fn editor_for_image_associations() {
    let m = map(&[("*.svg", "default"), ("*.dat", "imagePreview.previewEditor")]);
    assert_eq!(editor_for(Path::new("/r/i.svg"), Path::new("i.svg"), &m), FileView::Source);
    assert_eq!(editor_for(Path::new("/r/x.dat"), Path::new("x.dat"), &m), FileView::Image);
}
```

In `tests/editor_groups_model.rs`:
- `image_tab_opens_as_preview_and_reuses`:
  - `open_file(p, FileView::Image, OpenMode::Preview, true, true)` gives an unpinned `TabItem::ImagePreview(p)`.
  - Opening `p` again with `reuse_any` activates the same tab id.
  - `is_singleton()` is false.
- `image_tab_saves_and_restores`: the saved state path is `lucerna:image-preview:/abs/p.png`, and restoring it gives `ImagePreview`.
- `image_tab_follows_rename`: `retarget("/r/a.png", "/r/b.png")` changes the item path.

- [ ] **Step 2: Run them and see them fail to compile**

Run: `cargo test -p lucerna-workbench --test image_preview_info --test open_files --test editor_groups_model`
Expected: errors that `format_size`, `FileView::Image` and `TabItem::ImagePreview` are unknown.

- [ ] **Step 3: Implement**

`editor_for`:
- A glob match wins: `PREVIEW_EDITOR_ID` gives `Preview`, `IMAGE_EDITOR_ID` gives `Image`, anything else gives `Source`.
- With no match, `is_image_path(path)` gives `Image`.
- The early return `if map.is_empty() { return FileView::Source }` must go, so that the builtin image rule runs too.

Every `match` on `TabItem` in the crate must handle the new variant. Let the compiler list them. In `editor_groups/view.rs` and `workbench.rs`, put `todo!()`-free placeholders: the tab icon is `file(p)`, and the body factory returns an empty `div` view until Task 2. The model treats `ImagePreview` like `MarkdownPreview` for opening, reusing, saving and closed-tab history.

- [ ] **Step 4: Run the tests and see them pass**

Run the same command. Expected: all pass. Then run `cargo test -p lucerna-workbench`. Expected: no regressions, and `markdown_preview.rs` stays green.

- [ ] **Step 5: Commit**

`git commit -m "feat(image): image tab kind and editor association" -- <the paths above>`

---

### Task 2: The `ImagePreview` view (load, decode, draw, errors)

**Files:**
- Create: `crates/lucerna-workbench/src/image_preview/view.rs`
- Create: `crates/lucerna-workbench/src/image_preview/load.rs`
- Modify: `crates/lucerna-workbench/src/image_preview/mod.rs`
- Modify: `crates/lucerna-workbench/src/workbench.rs:~474` (tab body factory: build `ImagePreview`)
- Modify: `crates/lucerna-workbench/Cargo.toml` and the root `Cargo.toml` `[workspace.dependencies]`:
  - `image = { version = "0.25", default-features = false }`
  - `usvg = { version = "0.46", default-features = false }`
  - Check `Cargo.lock` gains no new package.
- Test: `crates/lucerna-workbench/tests/image_preview.rs` (new; copy the `init`/`draw`/`open` harness pattern from `tests/markdown_preview.rs`, and write fixtures with `image::RgbaImage::save`)

**Interfaces:**
- Consumes: `info::*`, `MAX_IMAGE_BYTES`, `TabItem::ImagePreview`.
- Produces in `load.rs`, which runs on the background executor and has no gpui window:
  ```rust
  pub enum LoadOutcome {
      Loaded { image: Arc<RenderImage>, dims: (u32, u32), bytes: u64 },
      TooLargeBytes(u64),
      TooLargePixels(u32, u32),
      LfsPointer { bytes: u64 },
      Failed { bytes: Option<u64> },
  }
  pub fn load_image(path: &Path, scale_factor: f32, svg: SvgRenderer) -> LoadOutcome
  ```
  - The order is: stat for size → read bytes → LFS check → dims check → decode.
  - Raster images:
    - Read the dims from the header with `image::ImageReader::with_guessed_format().into_dimensions()` before decoding.
    - Decode with `gpui::Image::from_bytes(format, bytes).to_image_data(svg)`, where `format` comes from `image::guess_format` mapped to `gpui::ImageFormat`.
  - SVG, which is a guess failure plus `is_svg_path`, or a guess failure that `usvg` parses:
    - Get the intrinsic size from `usvg::Tree::from_data(..).size()`.
    - The scale is `scale_factor`, lowered so that `w*h*scale²` ≤ `MAX_IMAGE_PIXELS`.
    - Rasterise with `svg.render_single_frame(&bytes, scale)`.
    - `dims` is the intrinsic size, rounded.
- Produces in `view.rs`:
  ```rust
  pub struct ImagePreview { /* path, state, focus, task, scale_factor */ }
  impl ImagePreview {
      pub fn new(path: Arc<Path>, window: &mut Window, cx: &mut Context<Self>) -> Self;
      pub fn path(&self) -> &Arc<Path>;
      pub fn dims(&self) -> Option<(u32, u32)>;
      pub fn file_bytes(&self) -> Option<u64>;
      pub fn is_loaded(&self) -> bool;
      pub fn error_message(&self) -> Option<SharedString>;
      pub fn render_image(&self) -> Option<Arc<RenderImage>>;   // for tests
  }
  impl Focusable, Render
  ```
  `new` starts `load_image` at once. The result is applied with `this.update`, and a dropped entity simply ignores it.

- [ ] **Step 1: Write the failing UI tests** in `tests/image_preview.rs`

Shared harness: a temp dir with `a.png` (3×2 RGBA). The workbench is opened with `open_path("a.png", OpenMode::Preview)`, then drawn.

- `png_opens_in_image_tab`:
  - The active tab is `TabItem::ImagePreview(abs a.png)` and unpinned.
  - The body downcasts to `ImagePreview`.
  - After `run_until_parked`, `dims() == Some((3, 2))` and `is_loaded()`.
- `svg_opens_as_image_with_intrinsic_size`:
  - `i.svg` = `<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"/>` gives `dims() == Some((40, 20))`.
  - The `render_image()` frame size equals `(40 × sf, 20 × sf)`, where `sf` is the window scale factor.
- `wrong_extension_still_decodes`: PNG bytes saved as `b.jpg` load with dims `(3, 2)`.
- `empty_file_shows_error`: empty `c.png` gives `error_message() == Some("The image could not be loaded.")`, and the `image-open-as-text` button exists (find it by `debug_selector`).
- `too_large_bytes`: a 21 MiB file gives the message `The image is too large to preview (21.0 MB).`
- `lfs_pointer_message`: gives `This image is stored with Git LFS and has not been downloaded.`
- `close_during_decode_is_harmless`: open, close the tab before `run_until_parked`, then `run_until_parked`. Expect no panic.
- `open_with_line_opens_source`: `open_path_at("i.svg", line 3)` gives `TabItem::File`. Use whatever entry point Quick Open uses for `:line`.

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -p lucerna-workbench --test image_preview`
Expected: compile errors, or failures because the body is still the placeholder.

- [ ] **Step 3: Implement `load.rs` and `view.rs`, and wire the factory**

Render rules:
- Loaded state:
  - A full-size flex container centred on `ui.editor_background`, with padding `GRID * 2`.
  - Inside, a box sized to `min(1, avail_w / w, avail_h / h) × (w, h)` in logical px.
  - The box has `gpui::checkerboard(<theme token for subtle bg>, 8.0)` as its background and a 1px `ui.border_variant` border.
  - `img(ImageSource::Render(image)).size_full()` fills the box.
  - Measure the available size the way `markdown_preview` measures its width (a canvas or `on_children_prepainted`). Do not hard-code it.
- Loading state: an empty background.
- Error state:
  - The same centred-message layout as `CodeView::render_error` (`code_view/mod.rs:~2147`), with the message and the `Open as Text` button (`debug_selector("image-open-as-text")`).
  - If that layout is not already shared, extract it into `ui.rs` as `pub fn centered_notice(message, button: Option<Stateful<Div>>) -> Div` and use it from both places.
  - The button calls `Workbench::reopen_image_as_text(tab, force_text: true)`, which Task 5 defines. Until then, this task stubs it as a method that replaces the item with `TabItem::File` and calls `Documents::open_as_text(path)` after `ensure_active_body`.

- [ ] **Step 4: Run the tests and see them pass**

Run: `cargo test -p lucerna-workbench --test image_preview`. Expected: PASS.

- [ ] **Step 5: Commit**

`git commit -m "feat(image): image preview view with background decode" -- <paths, Cargo.toml, Cargo.lock>`

---

### Task 3: Freeing images and SVG scale-factor changes

**Files:**
- Modify: `crates/lucerna-workbench/src/image_preview/view.rs`
- Test: `crates/lucerna-workbench/tests/image_preview.rs`

**Interfaces:**
- Produces:
  - `ImagePreview::release(&mut self, window: Option<&mut Window>, cx: &mut App)`, which calls `cx.drop_image(image, window)` on the held `Arc<RenderImage>` and clears it.
  - `impl Drop`-free design: call `release` when the image is replaced (Task 4) and when the entity is released, using `cx.on_release` in `new`.
- When the window's scale factor differs from the one the SVG was rasterised at (check it in `render`), the SVG is re-loaded at the new factor.

- [ ] **Step 1: Write the failing tests**
- `closing_tab_frees_image`:
  - Keep a `Weak` of `render_image()` before closing the tab.
  - After the close and `run_until_parked`, the weak is dead. That proves no cache holds it.
- `svg_rerasters_on_scale_change`:
  - Only if the test platform lets the scale factor change (check `TestWindow` for a scale-factor setter).
  - Otherwise, call a test-only `ImagePreview::set_scale_factor_for_test(2.0, cx)` and expect the frame size to double.

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -p lucerna-workbench --test image_preview closing_tab_frees_image svg_rerasters_on_scale_change`. Expected: FAIL.

- [ ] **Step 3: Implement** `release`, the `on_release` hook, and the scale-factor check.

- [ ] **Step 4: Run them and see them pass.** Expected: PASS.

- [ ] **Step 5: Commit**

`git commit -m "feat(image): free decoded images, re-raster svg on scale change"`

---

### Task 4: Reload on disk change, and the deleted state

**Files:**
- Modify: `crates/lucerna-workbench/src/image_preview/view.rs`
- Modify: `crates/lucerna-workbench/src/image_preview/mod.rs`: add `pub(crate) fn on_worktree_event(wb: &Workbench, ev: &WorktreeEvent, cx: &mut App)`, which finds every `ImagePreview` body (the same way `markdown_preview::commands::previews` does) and forwards the changes.
- Modify: `crates/lucerna-workbench/src/workbench.rs:~1561`: call it right after `documents.on_worktree_event`.
- Modify: `crates/lucerna-workbench/src/editor_groups/view.rs:~935`: the tab's struck-through title also comes from `ImagePreview::is_deleted()` when the body is an `ImagePreview`.
- Modify: `crates/lucerna-workbench/src/markdown_preview/commands.rs` (`retarget_previews`) or the rename path that calls it: the `ImagePreview` path follows renames through `ImagePreview::follow_rename(path, cx)`.
- Test: `crates/lucerna-workbench/tests/image_preview.rs`

**Interfaces:**
- Consumes: `WorktreeEvent::{Changed(Vec<Change>), Rescanned}`. `Change.path` is root-relative: join it to `wb.root()` before comparing with the absolute tab path.
- Produces on `ImagePreview`:
  - `pub fn file_changed(&mut self, kind: ChangeKind, cx: &mut Context<Self>)`
    - `Modified` or `Added` schedules a reload after `RELOAD_DEBOUNCE` (100 ms). A change during the wait restarts it.
    - `Removed` sets `deleted = true` and keeps the image.
    - `Added` after `Removed` clears `deleted` and reloads.
  - `pub fn is_deleted(&self) -> bool`
  - `pub fn follow_rename(&mut self, path: Arc<Path>, cx: &mut Context<Self>)`
  - `pub const RELOAD_DEBOUNCE: Duration = Duration::from_millis(100);`
  - `pub fn loads_started(&self) -> u64` (for tests)
- Reload rule:
  - A reload's `Loaded` replaces the image and releases the old one.
  - Any other outcome is ignored while an image is shown, so the old image stays and is not replaced by an error screen.
  - Only the first load may produce an error screen.
- Files outside `wb.root()` never get events, and nothing else watches them (spec §4.5).
- A reload never pins or unpins the tab.

- [ ] **Step 1: Write the failing tests**

Each test simulates `WorktreeEvent::Changed` the way `markdown_preview.rs` does.
- `rewrite_reloads_new_dims`: rewrite `a.png` as 5×4, send `Modified`, advance the clock by `RELOAD_DEBOUNCE`, park. Expect `dims() == Some((5, 4))`.
- `rewrites_in_a_burst_reload_once`: send 5 `Modified` events 20 ms apart, then wait for the debounce. Expect `loads_started()` to go up by 1.
- `failed_reload_keeps_old_image`: write garbage, send `Modified`, wait. Expect the dims to stay `(3, 2)` and `error_message()` to be `None`.
- `removed_marks_deleted_and_back_reloads`:
  - `Removed` gives `is_deleted()`, the image is still present, and the tab title is struck through (`debug_selector` `tab-deleted-0-0`).
  - Recreating the file and sending `Added` gives `!is_deleted()` and a reload.
- `reload_keeps_preview_tab_unpinned`: the tab opened with `OpenMode::Preview` is still unpinned after a reload.
- `rename_follows`: run the workspace rename path (the one `explorer_edit.rs` tests use). The tab item and `path()` show the new name.

- [ ] **Step 2: Run them and see them fail.** Expected: FAIL.

- [ ] **Step 3: Implement.**

- [ ] **Step 4: Run them and see them pass.** Expected: PASS. Then run the full `cargo test -p lucerna-workbench`.

- [ ] **Step 5: Commit**

`git commit -m "feat(image): reload on disk change, deleted state, follow rename"`

---

### Task 5: Commands, ⌘⇧V, context keys, path-bar button, Explorer menu

**Files:**
- Create: `crates/lucerna-workbench/src/image_preview/commands.rs`
- Modify: `crates/lucerna-workbench/src/commands.rs`, where the markdown `register_commands` is called: also call `image_preview::commands::register_commands`.
- Modify: `crates/lucerna-workbench/src/workbench.rs:~2566-2590` (context keys)
- Modify: `crates/lucerna-settings/assets/default-keybindings.json:185`
- Modify: `crates/lucerna-workbench/src/editor_groups/view.rs:~790` (`markdown_button`: add the SVG cases)
- Modify: `crates/lucerna-workbench/src/explorer/menu.rs:~65`
- Test: `crates/lucerna-workbench/tests/image_preview.rs`, `crates/lucerna-settings/tests/keymap.rs`

**Interfaces:**
- Produces:
  - `pub const REOPEN_AS_TEXT: &str = "imagePreview.reopenAsText";`
  - `pub const REOPEN_AS_PREVIEW: &str = "imagePreview.reopenAsPreview";`
  - `Workbench::reopen_image_as_text(&mut self, tab: TabId, force_text: bool, window, cx)`:
    - It runs `replace_item(tab, TabItem::File(path))`, which already activates an existing source tab instead of duplicating it.
    - With `force_text`, it calls `ensure_active_body` and then `documents.open_as_text(&path)`.
    - It focuses the editor area.
  - The reverse, `reopen_as_image_preview`, uses `replace_item(tab, TabItem::ImagePreview(path))`.
- Command availability: `REOPEN_AS_TEXT` is shown and runs only when the active tab is an `ImagePreview` of an SVG. `REOPEN_AS_PREVIEW` only when it is a `File` tab of an SVG. Follow how markdown hides commands per tab, if it does. If it doesn't, the commands do nothing on other tabs.
- Context keys:
  - `activeEditor` is `"imagePreview.previewEditor"` when the active body is an `ImagePreview`. It keeps `"markdownPreview"` and `""` for the existing cases.
  - `resourceExtname` is set from the image tab's path too.
- Keybindings, appended after line 185:
  ```json
  { "key": "cmd+shift+v", "command": "imagePreview.reopenAsPreview", "when": "resourceExtname == .svg && activeEditor == ''" },
  { "key": "cmd+shift+v", "command": "imagePreview.reopenAsText", "when": "resourceExtname == .svg && activeEditor == 'imagePreview.previewEditor'" },
  ```
  ⌘⇧V does nothing on raster image tabs, because no binding matches.
- Path-bar button:
  - On an SVG `File` tab: the `IMAGE` icon, tooltip `Reopen as Image Preview (<keys>)`.
  - On an SVG `ImagePreview` tab: the `FILE_CODE` icon, tooltip `Reopen as Text (<keys>)`.
  - Raster tabs have no button.
- Explorer:
  - The menu entry `Open Preview` on `.svg` files runs `REOPEN_AS_PREVIEW` with `{"path": rel}`.
  - With a path argument, it opens the file pinned as `FileView::Image`, like `markdown.showPreview` does for markdown.

- [ ] **Step 1: Write the failing tests**
- `cmd_shift_v_toggles_svg`:
  - Open `i.svg`, which gives `ImagePreview`.
  - ⌘⇧V gives `TabItem::File` with the same tab position and no extra tab.
  - ⌘⇧V again gives `ImagePreview`.
- `cmd_shift_v_noop_on_png`: the tab item is unchanged.
- `reopen_as_text_activates_existing_source`: with a group that already holds `File(i.svg)` and `ImagePreview(i.svg)`, `REOPEN_AS_TEXT` leaves a single tab, the `File` one, active.
- `open_as_text_button_forces_text`: on an empty `c.png`, click `image-open-as-text`. Expect a `CodeView` that is not in the Binary error state.
- `explorer_menu_has_open_preview_for_svg`: the menu for `i.svg` lists `Open Preview`, and the menu for `a.png` does not.
- In `lucerna-settings/tests/keymap.rs`: `svg_toggle_bindings_parse`. Both new entries load without errors, and `when` evaluates as expected for the two contexts.

- [ ] **Step 2: Run them and see them fail.** Expected: FAIL.

- [ ] **Step 3: Implement.** Replace the Task 2 stub of `reopen_image_as_text` with the real method.

- [ ] **Step 4: Run them and see them pass,** then run `cargo test -p lucerna-workbench -p lucerna-settings`.

- [ ] **Step 5: Commit**

`git commit -m "feat(image): svg reopen commands, cmd-shift-v, explorer entry"`

---

### Task 6: Status bar item

**Files:**
- Create: `crates/lucerna-workbench/src/image_preview/status.rs`
- Modify: `crates/lucerna-workbench/src/workbench.rs:~603`: create `ImageStatusItem` next to `DocumentStatusItem` and push it to `status_right`.
- Test: `crates/lucerna-workbench/tests/image_preview.rs`

**Interfaces:**
- Consumes: `EditorGroupsView::active_body()`, `ImagePreview::{dims, file_bytes}`, `info::status_text`.
- Produces: `pub(crate) struct ImageStatusItem { groups: Entity<EditorGroupsView> }` with `new(groups)` and `Render`.
  - It renders `div().debug_selector("status-image")` containing `status_text(dims, bytes)` when the active body is an `ImagePreview` and `file_bytes()` is `Some`. Otherwise it renders nothing.
  - It redraws when the view notifies. Mirror how `DocumentStatusItem` gets redrawn.
  - `DocumentStatusItem` already hides itself for bodies that are not a `CodeView`, so the text-editor items disappear on their own.

- [ ] **Step 1: Write the failing test**

`status_bar_shows_dims_and_size`:
- After `a.png` loads, the `status-image` element's text equals `status_text(Some((3, 2)), len)`, where `len` is the file length.
- The `status-cursor` element is absent.
- Rewriting as 5×4 and reloading updates the text.

- [ ] **Step 2: Run it and see it fail.** Expected: FAIL (no element).

- [ ] **Step 3: Implement.**

- [ ] **Step 4: Run it and see it pass.** Expected: PASS.

- [ ] **Step 5: Commit**

`git commit -m "feat(image): status bar dimensions and size"`

---

### Task 7: Docs, roadmap and the final check

**Files:**
- Modify: `docs/features.md`: an Image preview entry next to the Markdown one.
- Modify: `docs/keyboard-shortcuts.md`: ⌘⇧V on SVG.
- Modify: `docs/settings.md`: under `workbench.editorAssociations`, explain `imagePreview.previewEditor` and `"*.svg": "default"`.
- Modify: `docs/roadmap.md` and the roadmap table in `docs/superpowers/specs/2026-11-08-lucerna-phase1-design.md` §1.3: add an Image preview row, marked done.

- [ ] **Step 1: Write the docs.** English for `features.md`, `keyboard-shortcuts.md` and `settings.md`. For the roadmap and spec, match the language each file already uses.
- [ ] **Step 2: Run the full check**

```bash
scripts/prune-target.sh
```
```bash
cargo test -p lucerna-workbench -p lucerna-settings
```
```bash
cargo deny check
```
```bash
git diff main -- Cargo.lock
```

Expected: all tests pass. `deny` reports ok. The `Cargo.lock` diff only adds `image` and `usvg` to `lucerna-workbench`'s dependency list, with no new `[[package]]`.

- [ ] **Step 3: Manual check in the app** (with the `run` skill or `luc`):
  - A transparent PNG icon and an SVG logo look right in Lucerna Dark and Light.
  - The SVG is sharp on Retina.
  - Overwriting the PNG from a shell updates the tab without flashing.
  - Note the RAM before and after closing a 20 MB image.
  - Compare the release binary size with `main` (≤ 200 KB growth).
- [ ] **Step 4: Commit**

`git commit -m "docs: image preview" -- docs/features.md docs/keyboard-shortcuts.md docs/settings.md docs/roadmap.md docs/superpowers/specs/2026-11-08-lucerna-phase1-design.md`
