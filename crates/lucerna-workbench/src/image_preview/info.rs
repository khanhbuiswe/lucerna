//! Pure helpers of the image preview: which paths are images, the status
//! bar text and Git LFS pointer detection.

use std::path::Path;

/// The id `workbench.editorAssociations` uses for the image preview.
pub const IMAGE_EDITOR_ID: &str = "imagePreview.previewEditor";

/// Extensions opened as images, lowercase.
pub const IMAGE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "jpe", "gif", "webp", "bmp", "ico", "tif", "tiff", "svg",
];

/// Images with more pixels than this are not decoded.
pub const MAX_IMAGE_PIXELS: u64 = 100_000_000;

fn extension_is(path: &Path, is: impl Fn(&str) -> bool) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| is(&e.to_ascii_lowercase()))
}

/// Whether `path` has an image extension, ignoring case.
pub fn is_image_path(path: &Path) -> bool {
    extension_is(path, |e| IMAGE_EXTENSIONS.contains(&e))
}

/// Whether `path` has the `svg` extension, ignoring case.
pub fn is_svg_path(path: &Path) -> bool {
    extension_is(path, |e| e == "svg")
}

/// A byte count as `512 B`, `3.2 KB` or `34.2 MB` (base 1024, one decimal
/// for KB and MB).
pub fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * 1024;
    if bytes < KB {
        format!("{bytes} B")
    } else if bytes < MB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    }
}

/// The status bar text: `512 × 512 • 3.2 KB`, or only the size when the
/// dimensions are unknown.
pub fn status_text(dims: Option<(u32, u32)>, bytes: u64) -> String {
    match dims {
        Some((w, h)) => format!("{w} \u{d7} {h} \u{2022} {}", format_size(bytes)),
        None => format_size(bytes),
    }
}

/// Whether `bytes` is a Git LFS pointer file, not the image itself.
pub fn is_lfs_pointer(bytes: &[u8]) -> bool {
    bytes.starts_with(b"version https://git-lfs.github.com/spec/")
}
