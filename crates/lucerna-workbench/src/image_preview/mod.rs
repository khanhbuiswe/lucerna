//! Image preview: opens raster and SVG files in a tab instead of the text
//! editor. This module holds the pure helpers.

pub mod info;

pub use info::{
    IMAGE_EDITOR_ID, IMAGE_EXTENSIONS, MAX_IMAGE_PIXELS, format_size, is_image_path,
    is_lfs_pointer, is_svg_path, status_text,
};
