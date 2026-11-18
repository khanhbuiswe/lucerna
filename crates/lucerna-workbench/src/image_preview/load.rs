//! Reading and decoding an image file. Runs on the background executor:
//! no window, no entity.

use std::fs;
use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;

use gpui::{DevicePixels, Image, ImageFormat, RenderImage, SvgRenderer, SvgSize, size};
use image::codecs::gif::GifDecoder;
use image::codecs::webp::WebPDecoder;
use image::{AnimationDecoder, Frame, Rgba};

use super::info::{MAX_IMAGE_PIXELS, is_lfs_pointer};
use crate::markdown_preview::MAX_IMAGE_BYTES;

/// The longest side an SVG is rasterised to, in device px.
const MAX_SVG_SIDE: f64 = 8192.;

/// What reading an image file gave.
pub enum LoadOutcome {
    /// Decoded (an SVG: rasterised). `dims` is the pixel size as shown,
    /// after any EXIF orientation (an SVG's intrinsic size, rounded);
    /// `bytes` the file's length; `svg` whether it was rasterised from an
    /// SVG (so a new scale factor needs a reload).
    Loaded {
        image: Arc<RenderImage>,
        dims: (u32, u32),
        bytes: u64,
        svg: bool,
    },
    /// The file is larger than [`MAX_IMAGE_BYTES`]; not read.
    TooLargeBytes(u64),
    /// A raster image of more than [`MAX_IMAGE_PIXELS`] (width, height),
    /// in a file of `bytes`; not decoded.
    TooLargePixels(u32, u32, u64),
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
    // The header's size: the same area as the decoded one, which an EXIF
    // orientation may turn.
    let header = image::ImageReader::new(Cursor::new(&data))
        .with_guessed_format()
        .ok()
        .and_then(|reader| reader.into_dimensions().ok());
    let Some((w, h)) = header else {
        return failed;
    };
    if u64::from(w) * u64::from(h) > MAX_IMAGE_PIXELS {
        return LoadOutcome::TooLargePixels(w, h, bytes);
    }
    let image = match animation(guessed, &data) {
        Animation::Frames(frames) => Ok(Arc::new(RenderImage::new(frames))),
        Animation::Failed => return failed,
        Animation::Still => Image::from_bytes(format, data).to_image_data(svg),
    };
    let Ok(image) = image else {
        return failed;
    };
    // As shown: EXIF orientations 5 to 8 swap the sides.
    let shown = image.size(0);
    let dims = (shown.width.0.max(0) as u32, shown.height.0.max(0) as u32);
    LoadOutcome::Loaded {
        image,
        dims,
        bytes,
        svg: false,
    }
}

/// What decoding a file as an animation gave.
enum Animation {
    /// Its frames, BGRA.
    Frames(Vec<Frame>),
    /// An animated file none of whose frames decodes.
    Failed,
    /// Not an animation: gpui decodes it.
    Still,
}

/// Decodes a GIF or animated WebP here rather than in gpui, which keeps
/// every frame however many there are; [`animation_frames`] bounds them by
/// [`MAX_IMAGE_PIXELS`].
fn animation(format: image::ImageFormat, data: &[u8]) -> Animation {
    let frames = match format {
        image::ImageFormat::Gif => match GifDecoder::new(Cursor::new(data)) {
            Ok(decoder) => animation_frames(decoder, MAX_IMAGE_PIXELS),
            Err(_) => None,
        },
        image::ImageFormat::WebP => match WebPDecoder::new(Cursor::new(data)) {
            Ok(mut decoder) if decoder.has_animation() => {
                // Transparent where no frame has drawn yet, as gpui does.
                decoder.set_background_color(Rgba([0, 0, 0, 0])).ok();
                animation_frames(decoder, MAX_IMAGE_PIXELS)
            }
            Ok(_) => return Animation::Still,
            Err(_) => None,
        },
        _ => return Animation::Still,
    };
    frames.map_or(Animation::Failed, Animation::Frames)
}

/// Rasterises SVG `data` at `scale_factor` device pixels per CSS pixel,
/// lowered so the result has at most [`MAX_IMAGE_PIXELS`] and sides of at most 8192; `None` when it
/// does not parse.
fn load_svg(data: &[u8], scale_factor: f32, svg: &SvgRenderer) -> Option<LoadOutcome> {
    let tree = usvg::Tree::from_data(data, &usvg::Options::default()).ok()?;
    let (w, h) = (tree.size().width(), tree.size().height());
    let area = f64::from(w) * f64::from(h);
    let mut scale = f64::from(scale_factor.max(f32::MIN_POSITIVE));
    // gpui caps a side itself, with a warning and a blurry upscale.
    scale = scale.min(MAX_SVG_SIDE / f64::from(w.max(h)).max(1.));
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
        svg: true,
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

/// The frames of an animation, BGRA as gpui draws them, while frames ×
/// width × height stays within `budget` pixels; past it, only the first
/// frame (shown still). Frames that fail to decode are skipped, as gpui
/// does; `None` when none decodes.
fn animation_frames<'a>(decoder: impl AnimationDecoder<'a>, budget: u64) -> Option<Vec<Frame>> {
    let mut frames = Vec::new();
    let mut pixels = 0u64;
    for frame in decoder.into_frames().flatten() {
        let (w, h) = frame.buffer().dimensions();
        pixels += u64::from(w) * u64::from(h);
        if pixels > budget && !frames.is_empty() {
            // Too many to keep: the first frame, still.
            frames.truncate(1);
            break;
        }
        frames.push(frame);
    }
    for frame in &mut frames {
        for pixel in frame.buffer_mut().as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
    }
    (!frames.is_empty()).then_some(frames)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A GIF of `frames` frames of `w`×`h`.
    fn gif(w: u32, h: u32, frames: u8) -> Vec<u8> {
        let mut gif = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut gif);
            for i in 0..frames {
                let buffer = image::RgbaImage::from_pixel(w, h, image::Rgba([i * 40, 0, 0, 255]));
                encoder.encode_frame(Frame::new(buffer)).unwrap();
            }
        }
        gif
    }

    fn frames(data: &[u8], budget: u64) -> Vec<Frame> {
        animation_frames(GifDecoder::new(Cursor::new(data)).unwrap(), budget).unwrap()
    }

    #[test]
    fn animation_within_budget_keeps_every_frame() {
        let frames = frames(&gif(10, 10, 5), 500);
        assert_eq!(frames.len(), 5);
        // BGRA: the red channel moved to the third byte.
        assert_eq!(&frames[1].buffer().as_raw()[..4], &[0, 0, 40, 255]);
    }

    #[test]
    fn animation_over_budget_keeps_first_frame() {
        let frames = frames(&gif(10, 10, 5), 499);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].buffer().dimensions(), (10, 10));
        assert_eq!(&frames[0].buffer().as_raw()[..4], &[0, 0, 0, 255]);
    }
}
