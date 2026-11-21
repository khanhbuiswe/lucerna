# Image preview — design

Date: 2026-11-18. Status: approved.

## 1. Intent

The repo has many assets: icons, logos, screenshots, illustrations in the docs.
Users click an image file in Explorer or Quick Open to see what it looks like,
without switching to Finder or Preview.

Today Lucerna reports "The file appears to be binary." with an "Open as Text"
button when opening a `.png`. A `.svg` opens as source code.

The feature counts as successful when:
- Clicking an image file shows the image in the tab right away, with its size
  in pixels and its file size.
- When an agent overwrites the image file, the tab updates by itself.
- Closing the tab frees the image's memory.

## 2. Goals, non-goals, anti-goals

**Goals**
- Open PNG, JPEG, GIF (animated), WebP, BMP, TIFF, ICO and SVG files in an
  image tab.
- SVG opens as an image by default, like VS Code. A command, a button and
  ⌘⇧V switch back and forth with the source code.
- An image tab behaves like a file tab:
  - opens as a preview tab (italic title),
  - is kept by double-click,
  - can be split to the neighboring pane,
  - is saved and restored with the workspace,
  - follows the file when it is renamed.
- The image is centered. A large image is scaled down to fit the frame; a small
  image keeps its original size. A checkerboard background sits behind the
  image so transparent areas are easy to see.
- The status bar shows `W × H • file size`.
- When the file changes on disk, the tab reloads by itself.

**Non-goals (this version)**
- Zoom, dragging the image, viewing at the original size when the image is
  larger than the frame.
- Editing the image, copying the image (`imagePreview.copyImage`).
- Comparing images in a git diff (Uncommitted Changes, `git.openChange`).
- Video and audio. Formats GPUI cannot read (AVIF, TGA, HDR, EXR…).
- Images in hover and in Quick Open.

**Anti-goals** (it runs but still counts as a failure):
- The UI thread stalls while reading or decoding a large image.
- Closing a tab does not reduce RAM, or constant reloading makes RAM grow
  steadily. Measured (2026-11-18): closing a tab frees the GPU texture and the
  image data; the remaining footprint is libmalloc's large cache on macOS, does
  not grow across reopens, and is smaller than both Zed and VS Code. Details
  and the decision to accept it are in
  `docs/research/2026-11-18-image-preview.md` §6.
- The old image still shows after the file on disk has changed.
- The image flashes white every time an agent rewrites the file.
- A new crate is added to `Cargo.lock`, or the binary grows by more than
  200 KB.
- The text-handling path is touched (`Documents`, saving files, hot exit).

## 3. References

Details are in `docs/research/2026-11-18-image-preview.md`. Summary:

| Aspect | VS Code | Zed | Lucerna |
|---|---|---|---|
| SVG default | image | text | **VS Code** (image) |
| Editor choice | `imagePreview.previewEditor`, builtin priority, changed via `workbench.editorAssociations` | fixed list of extensions | **VS Code** |
| Switch SVG ↔ source | `imagePreview.reopenAsText` / `reopenAsPreview`, button in the title | none | **VS Code** commands, **Zed**-style button (icon on the breadcrumb bar, like markdown), plus ⌘⇧V (a deviation, §4.2) |
| Size when opened | fit the frame, shrink only | same as VS Code | same |
| Background | 16px checkerboard behind the image, 1px border | `gpui::checkerboard`, 1px border | **Zed** (`gpui::checkerboard`) |
| Status bar | zoom, `WxH`, KB | `WxH • size • channels • format` | **Zed** in form, **VS Code** in content: `W × H • size` |
| Reload | watcher, keeps the old image on error | re-reads the bytes, replaces only after decoding, frees the old image | **Zed** |
| Helix | none | | |

## 3b. Options considered

1. **Add a separate image tab kind** (`TabItem::ImagePreview`,
   `FileView::Image`), not going through `Documents`. **The user chose this
   option.** It does not touch the markdown preview, and the image stays
   separate from the text path. The cost is repeating a little of the
   switch-between-source-and-preview logic.
2. **Merge with markdown into one "rendered" kind.** The switching logic lives
   in one place, but the markdown preview has to be changed (dynamic preview,
   scroll sync), which images do not need. Likely to break a feature that works
   fine today.
3. **Draw the image into the "binary" screen of `Documents`.** The least code,
   but it mixes images into the text path (encoding, saving, hot exit). SVG
   never reaches that branch. The status bar information is also hard to attach.

## 4. Decisions

### 4.1 Which files open as an image

Add `FileView::Image` to the enum that chooses between source and preview. The
`editor_for` function (`markdown_preview/commands.rs`) returns:
1. When a glob in `workbench.editorAssociations` matches: `Image` if the id is
   `imagePreview.previewEditor`, `Source` if the id is `default`, and `Preview`
   for markdown as today.
2. When no glob matches and the file extension (case-insensitive) is in the
   image list: `Image`. This is VS Code's `builtin` priority, so the image
   extensions do not need to be added to the setting's default value.

Image extension list: `png jpg jpeg jpe gif webp bmp ico tif tiff svg`.

Anyone who wants SVG to open as source code as before sets
`"workbench.editorAssociations": { "*.svg": "default" }`. The same works for
other extensions, but raster images then land on the existing "binary" screen.

Every path that opens a file goes through here: Explorer, Quick Open, Open
Recent, `luc file`, restored sessions. Opening with a line number
(`file.svg:12`, Go to Definition, search results) still opens source code, as
`.md` does today.

### 4.2 Switching between SVG and source

- **Commands** (following VS Code):
  - `imagePreview.reopenAsText` ("Reopen as Text"): replaces the open image tab
    with a source tab at the same position.
  - `imagePreview.reopenAsPreview` ("Reopen as Image Preview"): does the
    reverse.
  - Shown in the command palette only when the current tab is an SVG.
- **Button:** an icon on the breadcrumb bar, in the same place and style as
  markdown's preview button.
- **Key:** ⌘⇧V invokes whichever command is the other one. VS Code assigns no
  key for this. Lucerna assigns it because ⌘⇧V is already familiar from
  markdown and the user chose this approach. This is a deliberate deviation.
  - How it is bound: two keybinding rows for ⌘⇧V, each with a `when` condition
    on the tab kind (SVG as source, SVG as image). The markdown row stays
    unchanged.
  - On a raster image tab, ⌘⇧V does nothing.
  - *(Claude decided.)* The switch always happens in place, as
    `markdown.togglePreview` does today. If the group already has a tab for the
    other form of the same file, that tab is activated instead of creating a
    duplicate, the way markdown handles it today.
- **Explorer:** the right-click menu of a `.svg` file has "Open Preview", like
  `.md`.
- Raster images have none of the commands above. To view one as text, use the
  "Open as Text" button on the error screen (§4.6).

### 4.3 Tabs

- Add `TabItem::ImagePreview(Arc<Path>)`. Not a singleton: any number can be
  opened, like `File` and `MarkdownPreview`.
- The tab title is the file name, with an icon by file type as in file tabs.
- Preview tab, pinned tab, open in the neighboring pane: same rules as file
  tabs. Within a group, reopening a file that already has an image tab
  activates that tab.
- Stored in `GroupState::tabs` with the prefix `lucerna:image-preview:` plus
  the absolute path, like `MARKDOWN_PREVIEW_STATE_PREFIX`.
- When the file is renamed: the tab follows the new name, like file tabs and
  markdown tabs.
- When the file is deleted: the tab keeps the last image, and the tab title is
  struck through like a file tab. There is no Close/Keep bar, because that bar
  takes its state from `Documents`. When the file reappears, the tab reloads the
  image and the strikethrough is removed. *(Claude decided.)*
- Reloading is not a user action. A preview tab (italic title) is still a
  preview tab after a reload.
- An image tab is never dirty. ⌘S does nothing. Editing commands (find, go to
  line, format…) do not apply to this tab.

### 4.4 Image view

- A new `ImagePreview` entity lives in `lucerna-workbench`, in the
  `image_preview/` module. This view does not use `Documents`.
- **Loading:** read the bytes and decode in the background, using
  `Image::from_bytes`. The format is guessed from the bytes; if it cannot be
  guessed and the extension is `.svg`, try SVG. The pixel size is read at the
  same time. Draw with `img(ImageSource::Image(Arc<Image>))` so the image id
  follows the content, not the path.
- While the first load is in progress, the frame stays empty, with no spinner,
  like the markdown preview.
- **Layout:**
  - The image sits in the middle of the frame. The scale is
    `min(frame / image, 1)` in both dimensions.
  - The frame has a margin of a few pixels, using the same spacing tokens as
    the other views.
  - The frame background is the editor background color. Only the area right
    under the image gets `gpui::checkerboard` with colors taken from the theme,
    plus a 1px border.
  - When the frame is resized, the image scale is recomputed.
- **SVG** is drawn at the exact resolution for the displayed size and the
  display's scale factor, so it is not blurry on Retina displays. The SVG's
  "original" size comes from the file's `width`/`height`/`viewBox`.
  - *(Claude decided.)* At the GPUI revision currently pinned,
    `Image::from_bytes` with an SVG always rasterizes at scale 1.0, so it would
    be blurry on Retina. SVG therefore has its own raster path:
    - Read the original size with `usvg`, a crate already in `Cargo.lock` via
      GPUI (add it as a direct dependency, exactly version 0.46).
    - Rasterize with GPUI's `SvgRenderer::render_single_frame(bytes, scale)`,
      where `scale` is the window's scale factor.
    - Because the image is only ever scaled down, never up, one raster at the
      original size times the scale factor is always sharp enough. There is no
      need to re-rasterize when the frame is resized, only when the scale
      factor changes (for example dragging the window to another display).
    - If the raster exceeds the megapixel limit in §4.6, lower `scale` to fit
      instead of reporting an error.
- An animated GIF plays, thanks to GPUI.

### 4.5 Reload and memory

- Watch the file through the workbench's existing `WorktreeEvent`. This event
  covers only the open folder. An image outside the workspace
  (`luc file /somewhere/else.png`, a restored session) is read once and does not
  reload by itself. *(Claude decided.)*
- When the tab's file changes on disk, re-read the bytes after a 100 ms
  debounce, as VS Code does. If the file is written continuously, only the last
  write is read.
- Replace the image only after decoding finishes. If reading or decoding fails
  during a reload, keep the old image and do not switch to the error screen (the
  file may be half-written). The next change will try again.
- When the image is replaced or the tab is closed, free the old image right away
  with `window.drop_image` and `remove_asset`.
- The bytes are kept only while decoding, then discarded.

### 4.6 Limits and errors

- A file larger than `MAX_IMAGE_BYTES` (20 MB, shared with the markdown
  preview) is not read. An image whose `width × height` is over 100 megapixels
  is not decoded either, because its RGBA takes over 400 MB of RAM.
- In both cases, and when decoding fails, the frame shows a centered message
  line (in English) with a button:
  - `The image is too large to preview (34.2 MB).`
  - `The image is too large to preview (20000 × 12000).`
  - `The image could not be loaded.`
  - The "Open as Text" button opens a source tab with `force_text`. This reuses
    the existing `Documents::open_as_text`.
  - The message and button use the components already in `ui.rs`, like the
    current binary screen.
- A Git LFS pointer file (a text file starting with
  `version https://git-lfs.github.com/spec/`) shows:
  `This image is stored with Git LFS and has not been downloaded.`

### 4.7 Status bar

- When an image tab is selected, the text editor's items (cursor position,
  encoding, line ending, language) are hidden. In their place is one item on the
  right: `512 × 512 • 3.2 KB`.
- File size: base 1024, written as `B`, `KB`, `MB` with one decimal place. The
  notation matches VS Code; Lucerna has no `unit` setting.
- While loading has not finished, only the file size is shown. On an error,
  only the file size is shown too.

## 5. Testing

- **Unit** (no GPUI needed):
  - `editor_for` with the extension list, with the `default` and
    `imagePreview.previewEditor` associations, with an uppercase extension.
  - The `W × H • size` format.
  - The byte and megapixel limits.
  - LFS pointer file detection.
  - Saving and restoring the `lucerna:image-preview:` prefix.
- **UI** (`gpui::TestAppContext`):
  - Opening a `.png` from Explorer gives an image tab as a preview tab.
  - Opening a `.svg` gives an image; ⌘⇧V switches to source and back.
  - Opening `file.svg:3` gives source code.
  - Overwriting the file replaces the old image with the new one and the size in
    the status bar changes accordingly.
  - Writing a corrupt file still keeps the old image.
  - Closing the tab frees the image.
  - The image tab is restored after reopening the workspace.
- **Manual checks:**
  - A transparent image displays correctly on both light and dark themes.
  - SVG is not blurry on a Retina display.
  - RAM drops back after closing a 20 MB image.

## 6. Left to the implementer

- Which theme tokens supply the checkerboard cell size and colors.
- How to split the code between `image_preview/` and
  `markdown_preview/commands.rs`. The `editor_for` function can move to a shared
  module if that is cleaner.
- Whether to use `RetainAllImageCache` for the view or manage the `Arc<Image>`
  directly, as long as §4.5 is met.
