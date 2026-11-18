//! The pure helpers of the image preview.

use std::path::Path;

use lucerna_workbench::{format_size, is_image_path, is_lfs_pointer, is_svg_path, status_text};

#[test]
fn format_size_units() {
    assert_eq!(format_size(512), "512 B");
    assert_eq!(format_size(3277), "3.2 KB");
    assert_eq!(format_size(34 * 1024 * 1024 + 200 * 1024), "34.2 MB");
}

#[test]
fn status_text_joins_dims_and_size() {
    assert_eq!(
        status_text(Some((512, 512)), 3277),
        "512 \u{d7} 512 \u{2022} 3.2 KB"
    );
    assert_eq!(status_text(None, 512), "512 B");
}

#[test]
fn image_paths() {
    assert!(is_image_path(Path::new("/a/logo.PNG")));
    assert!(is_image_path(Path::new("x.jpe")));
    assert!(!is_image_path(Path::new("x.avif")));
    assert!(is_svg_path(Path::new("i.SVG")));
}

#[test]
fn lfs_pointer() {
    assert!(is_lfs_pointer(
        b"version https://git-lfs.github.com/spec/v1\noid sha256:ab\n"
    ));
    assert!(!is_lfs_pointer(b"\x89PNG"));
}
