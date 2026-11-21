# Research: Image preview

Date 2026-11-18. Sources:
- VS Code: `microsoft/vscode` branch `main`. Commit `e7d9908` (2026-10-20, #341900) removed the `extensions/media-preview` extension and moved the image viewer into core, at `src/vs/workbench/contrib/mediaPreview/` (`common/mediaPreview.ts`, `mediaPreview.contribution.ts`, `browser/imageViewer.ts`, `browser/imagePreviewEditor.ts`, `browser/mediaPreviewStatusBar.ts`, `media/*.css`). IDs and behavior are unchanged. The old extension version is at commit `3f98d21`.
- Zed: the GPUI revision currently pinned, `b5cd310`, the `image_viewer` crate (`image_viewer.rs`, `image_info.rs`) and `project/src/image_store.rs`, plus `gpui` (`platform.rs`, `color.rs`, `elements/img.rs`). Zed is mostly GPL: only the design is recorded, no code is copied.
- Helix: `helix-term/src/ui/picker.rs`, `helix-term/src/commands.rs`.

## 1. VS Code

- **Editor:** id `imagePreview.previewEditor`, priority `builtin`. Takes files matching the glob `*.{jpg,jpe,jpeg,png,bmp,gif,ico,webp,avif,svg}`. **SVG opens as an image by default.** Users change this through `workbench.editorAssociations`.
- **Switching with source:** when viewing an `.svg` as an image, the editor title bar has `imagePreview.reopenAsText`. When viewing as text there is `imagePreview.reopenAsPreview`. Both call "Reopen Editor With". No default shortcut.
- **Size on open:** `fit` mode (`max-width/height: 100%`). A large image shrinks to fit the frame; a small image is not enlarged.
- **Zoom:**
  - Click to zoom in, Alt-click (macOS) to zoom out.
  - Mouse wheel with Alt or Ctrl, or a trackpad pinch, also zooms.
  - Fixed zoom levels from 0.1 to 20. From 3× up the image is drawn `pixelated`.
  - The commands `imagePreview.zoomIn` and `imagePreview.zoomOut` have no default keys.
- **Status bar (right side):**
  - Zoom level (`Whole Image` or `NN%`), clicking it opens a list to pick a level.
  - Size `WxH`.
  - File size `N.NNKB` (base 1024, 2 decimal places).
- **Background:** a 16px `conic-gradient` checkerboard, only behind the image. Color rgb(230) with a light theme, rgb(20) with a dark theme. The image has a 1px border.
- **Reload:** watches the file, debounce 100 ms, rereads the bytes, keeps the zoom and scroll position. If the read fails midway (the file is being written), the old image is kept.
- **Error:** shows "An error occurred while loading the image." in the view. The old version also had a link to open in a text editor. A Git LFS pointer file has its own message.
- **Settings:** there are no settings for images.
- **Other commands:** `imagePreview.copyImage`.

## 2. Zed

- **Files that open as images:** the extensions in `gpui::Img::extensions()` (avif, jpg, jpeg, png, gif, webp, tif, tiff, tga, dds, bmp, ico, hdr, exr, pnm…, qoi, svg), **except SVG: SVG opens as text.**
- **Size on open:** `min(scale_x, scale_y, 1.0)`, meaning it only shrinks, never enlarges.
- **Zoom and dragging the image:**
  - ⌘ with the mouse wheel zooms around the pointer. A plain wheel or dragging the mouse moves the image.
  - Keys: ⌘= and ⌘- to zoom, ⌘0 back to 100%, ⌘1 view at exact original size, ⌘⇧0 fit to frame.
  - There is a dedicated toolbar for zoom.
- **Background:** `gpui::checkerboard(panel_background, 32 * zoom)`, the checker cell size scales with zoom. The image has a 1px border.
- **Status bar (`ImageInfo`):** parts joined with " • ", for example `1920x1080 • 1.2 MiB • 4 channels, 8 bits per pixel • PNG`.
- **Settings:** `image_viewer.unit` takes `binary` (default, KiB) or `decimal` (KB).
- **Loading the image:** `ImageItem` holds an `Arc<gpui::Image>` built from bytes. The format is guessed with `image::guess_format`, and if that fails it tries SVG. The view decodes in the background before drawing so it does not flicker.
- **Reload:** when the worktree reports the file changed, `ReloadNeeded` fires, the bytes are reread, and the image is swapped only after decoding finishes. Then the old image is released with `window.drop_image` and `Image::remove_asset`. If the reload fails, the old image is kept and only a log line is written.
- **Tab:** the title is the file name, colored by git status. The icon follows the file type.

## 3. Helix

Helix cannot view images. The picker shows `<Binary file>`. Opening a `file:` URL that points to a binary file makes Helix hand it to the operating system's default program.

## 4. GPUI at the pinned revision

- `ImageFormat` has Png, Jpeg, Webp, Gif, Svg, Bmp, Tiff, Ico, Pnm. GIFs animate.
- `img(path)` caches by path. **Overwriting the file does not invalidate the cache**, so the old image keeps showing.
- Two ways to force a reload:
  - Use `ImageSource::Image(Arc<Image>)` with `Image::from_bytes`. The image id is the hash of the bytes, so new content automatically gets a new cache key.
  - Or call `remove_asset` and then `window.drop_image`.
- Lucerna's Markdown preview currently reloads images by recreating the `RetainAllImageCache`.
- `gpui::checkerboard(color, size)` is available in `color.rs`.

## 5. Comparison

| | VS Code | Zed | Helix |
|---|---|---|---|
| SVG by default | Image | Text | n/a |
| Size / zoom | Fit to frame, only shrinks; zoom 0.1–20 | Fit to frame, only shrinks; zoom, drag, keys | none |
| Status bar | Zoom, `WxH`, file size | `WxH • file size • channels • format` | none |
| Checkerboard background | Yes, fixed 16px | Yes, 32px scaled by zoom | none |
| Reload | Watcher, 100 ms, keeps zoom | On disk change reread, swap after decoding finishes | none |
| Settings | None | `image_viewer.unit` | none |

## 6. RAM after closing a large image (measured 2026-11-18)

Machine: macOS 26.7.1. Each app opens the same 5000×5000 PNG (about 100 MB when
decoded to RGBA), then the tab is closed. The numbers are physical footprint
(`footprint`, `vmmap`). VS Code is the total of all processes.

| | Initial | Open | After closing the tab | Still retained |
|---|---|---|---|---|
| Lucerna (debug build, `dc0f728`) | 85 MB | 411 MB | 317 MB | about 230 MB |
| Zed 1.28.1 | 158 MB | 443 MB | 442 MB | about 284 MB |
| VS Code 1.146 | 733 MB | 1162 MB | 1033 MB (after 40 s) | about 300 MB |

**What each app retains:**
- **Lucerna:**
  - GPU texture (IOAccelerator) is back to 4 MB.
  - Image data still alive on the CPU (`MALLOC_LARGE`) is back to 0.
  - The remainder is about 190 MB of `MALLOC_LARGE (empty)`: two buffers of about 95 MB produced during decoding. The app has `free`d them, but libmalloc still keeps them in its large cache.
  - Opening and closing again does not grow it further, because the cache is reused.
- **Zed:** keeps both the GPU texture (103 MB) and the same 190 MB large cache as Lucerna. Zed has `mimalloc` as a `#[global_allocator]`, but only as an optional feature that is off by default (`crates/zed/src/main.rs`).
- **VS Code:** the Renderer process returns its RAM after GC. Chromium's GPU process still holds about 316 MB.

**Cause on the Lucerna side** (verified with a small C program):
- `malloc` then `free` 100 MB: the footprint still holds 100 MB, even after 20 s.
- Freeing 300 MB: the footprint still stays the same.
- `malloc_zone_pressure_relief` returns nothing.
- Running with `MallocLargeCache=0`, the footprint goes to 0 right after the free.

This is libmalloc's large-cache policy on macOS, not a leak in the image
preview.

**Decision:** accept.
- Lucerna has released everything it owns, and still retains less than both Zed and VS Code.
- Options considered and left alone:
  - A `#[global_allocator]` that routes blocks from about 16 MiB up to `mmap`/`munmap`. This touches the whole program's allocator, so if it is done it needs its own spec.
  - Setting `MallocLargeCache=0` through `LSEnvironment`. This only takes effect when the app is opened through LaunchServices, and it turns off the cache for every large block.
- Markdown preview with a large image goes through the same decoding path, so it has the same level of retention.
