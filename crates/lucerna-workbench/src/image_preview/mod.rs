//! Image preview: opens raster and SVG files in a tab instead of the text
//! editor. [`info`] holds the pure helpers, [`load`] reads and decodes a
//! file in the background, [`view`] draws it.

pub mod info;
pub mod load;
pub mod view;

pub use info::{
    IMAGE_EDITOR_ID, IMAGE_EXTENSIONS, MAX_IMAGE_PIXELS, format_size, is_image_path,
    is_lfs_pointer, is_svg_path, status_text,
};
pub use load::{LoadOutcome, load_image};
pub use view::ImagePreview;
