//! Reading and decoding an image file. Runs on the background executor:
//! no window, no entity.

use std::fs;
use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;

use gpui::{DevicePixels, Image, ImageFormat, RenderImage, SvgRenderer, SvgSize, size};

use super::info::{MAX_IMAGE_PIXELS, is_lfs_pointer};
use crate::markdown_preview::MAX_IMAGE_BYTES;

/// What reading an image file gave.
pub enum LoadOutcome {
    /// Decoded (an SVG: rasterised). `dims` is the pixel size (an SVG's
    /// intrinsic size, rounded); `bytes` the file's length.
    Loaded {
        image: Arc<RenderImage>,
        dims: (u32, u32),
        bytes: u64,
    },
    /// The file is larger than [`MAX_IMAGE_BYTES`]; not read.
    TooLargeBytes(u64),
    /// A raster image of more than [`MAX_IMAGE_PIXELS`]; not decoded.
    TooLargePixels(u32, u32),
    /// A Git LFS pointer, not the image.
    LfsPointer { bytes: u64 },
    /// Unreadable, or not an image this preview decodes. `bytes` is the
    /// file's length when it could be read.
    Failed { bytes: Option<u64> },
}

/// Reads and decodes `path`: its size, then its bytes, then whether it is
/// an LFS pointer, then its pixel size, then the pixels. The format comes
/// from the bytes, not the extension; bytes in no raster format are tried
/// as SVG, rasterised at `scale_factor` (lowered to stay within
/// [`MAX_IMAGE_PIXELS`]).
pub fn load_image(path: &Path, scale_factor: f32, svg: SvgRenderer) -> LoadOutcome {
    let Ok(meta) = fs::metadata(path) else {
        return LoadOutcome::Failed { bytes: None };
    };
    if meta.len() > MAX_IMAGE_BYTES {
        return LoadOutcome::TooLargeBytes(meta.len());
    }
    let Ok(data) = fs::read(path) else {
        return LoadOutcome::Failed {
            bytes: Some(meta.len()),
        };
    };
    let bytes = data.len() as u64;
    if is_lfs_pointer(&data) {
        return LoadOutcome::LfsPointer { bytes };
    }
    let failed = LoadOutcome::Failed { bytes: Some(bytes) };
    let Ok(guessed) = image::guess_format(&data) else {
        // Not a raster format: an SVG (whatever its extension, so a
        // `.svg` and an SVG saved under another name both work) or nothing.
        return load_svg(&data, scale_factor, &svg).unwrap_or(failed);
    };
    let Some(format) = gpui_format(guessed) else {
        return failed;
    };
    let dims = image::ImageReader::new(Cursor::new(&data))
        .with_guessed_format()
        .ok()
        .and_then(|reader| reader.into_dimensions().ok());
    let Some((w, h)) = dims else {
        return failed;
    };
    if u64::from(w) * u64::from(h) > MAX_IMAGE_PIXELS {
        return LoadOutcome::TooLargePixels(w, h);
    }
    match Image::from_bytes(format, data).to_image_data(svg) {
        Ok(image) => LoadOutcome::Loaded {
            image,
            dims: (w, h),
            bytes,
        },
        Err(_) => failed,
    }
}

/// Rasterises SVG `data` at `scale_factor` device pixels per CSS pixel,
/// lowered so the result has at most [`MAX_IMAGE_PIXELS`]; `None` when it
/// does not parse.
fn load_svg(data: &[u8], scale_factor: f32, svg: &SvgRenderer) -> Option<LoadOutcome> {
    let tree = usvg::Tree::from_data(data, &usvg::Options::default()).ok()?;
    let (w, h) = (tree.size().width(), tree.size().height());
    let area = f64::from(w) * f64::from(h);
    let mut scale = f64::from(scale_factor.max(f32::MIN_POSITIVE));
    if area * scale * scale > MAX_IMAGE_PIXELS as f64 {
        scale = (MAX_IMAGE_PIXELS as f64 / area).sqrt();
    }
    let px = |v: f32| DevicePixels(((f64::from(v) * scale).round() as i32).max(1));
    let parsed = svg.parse_svg(data).ok()?;
    // An exact size: `render_single_frame` would rasterise at twice the
    // scale asked for.
    let image = svg
        .render_parsed(&parsed, SvgSize::ExactSize(size(px(w), px(h))))
        .ok()?;
    let dims = ((w.round() as u32).max(1), (h.round() as u32).max(1));
    Some(LoadOutcome::Loaded {
        image,
        dims,
        bytes: data.len() as u64,
    })
}

/// The gpui decoder for a format `image` recognised, if gpui decodes it.
fn gpui_format(format: image::ImageFormat) -> Option<ImageFormat> {
    Some(match format {
        image::ImageFormat::Png => ImageFormat::Png,
        image::ImageFormat::Jpeg => ImageFormat::Jpeg,
        image::ImageFormat::Gif => ImageFormat::Gif,
        image::ImageFormat::WebP => ImageFormat::Webp,
        image::ImageFormat::Bmp => ImageFormat::Bmp,
        image::ImageFormat::Tiff => ImageFormat::Tiff,
        image::ImageFormat::Ico => ImageFormat::Ico,
        _ => return None,
    })
}
