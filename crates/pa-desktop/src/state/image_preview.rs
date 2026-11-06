//! Decode bounded previews on a worker. Drafts retain only their small thumbnail.

use std::io::Cursor;

use anyhow::{Context as _, Result, ensure};
use image::metadata::Orientation;
use image::{DynamicImage, ImageDecoder as _, ImageFormat, ImageReader, Limits};
use rune::Pixels;

const THUMBNAIL_EDGE: u32 = 128;
const EXPANDED_EDGE: u32 = 1280;
const MAX_DIMENSION: u32 = 16_384;
const MAX_PIXELS: u64 = 32 * 1024 * 1024;
const MAX_DECODE_BYTES: u64 = 192 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct ImagePreview {
    pub thumbnail: Pixels,
    /// Full image dimensions as displayed, after applying the file's orientation.
    pub width: u32,
    pub height: u32,
}

/// Stage a thumbnail of the first frame and its original display dimensions.
pub(crate) fn decode(bytes: &[u8], mime: &str) -> Result<ImagePreview> {
    let (thumbnail, [width, height]) = decode_scaled(bytes, mime, THUMBNAIL_EDGE)?;
    Ok(ImagePreview {
        thumbnail,
        width,
        height,
    })
}

/// Decode the larger raster only for the image currently open in the preview window.
pub(crate) fn decode_expanded(bytes: &[u8], mime: &str) -> Result<Pixels> {
    decode_scaled(bytes, mime, EXPANDED_EDGE).map(|(pixels, _)| pixels)
}

fn decode_scaled(bytes: &[u8], mime: &str, edge: u32) -> Result<(Pixels, [u32; 2])> {
    let format = match mime {
        "image/png" => ImageFormat::Png,
        "image/jpeg" => ImageFormat::Jpeg,
        "image/gif" => ImageFormat::Gif,
        "image/webp" => ImageFormat::WebP,
        _ => anyhow::bail!("Unsupported image format for preview."),
    };
    if format == ImageFormat::WebP {
        validate_webp(bytes)?;
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(MAX_DECODE_BYTES);

    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(limits.clone());
    let mut decoder = reader.into_decoder().context("Cannot read image header")?;
    let (width, height) = decoder.dimensions();
    ensure!(
        width > 0 && height > 0 && u64::from(width) * u64::from(height) <= MAX_PIXELS,
        "Image exceeds the preview dimensions limit or has no pixels."
    );
    // Reserve space for the output buffer before letting the codec allocate working memory.
    limits.reserve(decoder.total_bytes())?;
    decoder.set_limits(limits)?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let (width, height) = match orientation {
        Orientation::Rotate90
        | Orientation::Rotate90FlipH
        | Orientation::Rotate270
        | Orientation::Rotate270FlipH => (height, width),
        _ => (width, height),
    };

    let mut raster = DynamicImage::from_decoder(decoder).context("Cannot decode image")?;
    if width.max(height) > edge {
        raster = raster.thumbnail(edge, edge);
    }
    // Rotate the bounded preview instead of allocating a second full-size camera image.
    raster.apply_orientation(orientation);
    Ok((pixels(raster), [width, height]))
}

// The WebP codec may allocate metadata using the declared RIFF chunk length, and
// checks a VP8 frame against its enclosing canvas only *after* decoding the frame.
// Bound those lengths and dimensions before handing the container to the codec.
fn validate_webp(bytes: &[u8]) -> Result<()> {
    let header = bytes
        .first_chunk::<12>()
        .context("Incomplete WebP header")?;
    ensure!(
        &header[..4] == b"RIFF" && &header[8..] == b"WEBP",
        "Invalid WebP header."
    );
    let length = u32::from_le_bytes(header[4..8].try_into()?);
    ensure!(
        u64::from(length) + 8 == bytes.len() as u64,
        "Invalid WebP container length."
    );
    check_webp_chunks(&bytes[12..], false)
}

fn check_webp_chunks(mut chunks: &[u8], in_frame: bool) -> Result<()> {
    while !chunks.is_empty() {
        let header = chunks.first_chunk::<8>().context("Incomplete WebP chunk")?;
        let size = u32::from_le_bytes(header[4..8].try_into()?);
        let length = usize::try_from(size)?;
        let (data, rest) = chunks[8..]
            .split_at_checked(length)
            .context("Invalid WebP chunk length")?;
        match &header[..4] {
            b"ANMF" => {
                ensure!(!in_frame, "Invalid nested WebP frame.");
                check_webp_chunks(
                    data.get(16..).context("Incomplete WebP animation frame")?,
                    true,
                )?;
            }
            b"VP8 " => {
                let frame = data.first_chunk::<10>().context("Incomplete WebP frame")?;
                let width = u16::from_le_bytes([frame[6], frame[7]]) & 0x3fff;
                let height = u16::from_le_bytes([frame[8], frame[9]]) & 0x3fff;
                ensure!(
                    u64::from(width) * u64::from(height) <= MAX_PIXELS,
                    "WebP frame exceeds the preview dimensions limit."
                );
            }
            _ => {}
        }
        chunks = rest
            .get(length % 2..)
            .context("Missing WebP chunk padding")?;
    }
    Ok(())
}

fn pixels(image: DynamicImage) -> Pixels {
    let rgba = image.into_rgba8();
    let pixels = Pixels::new();
    pixels.set(rgba.width(), rgba.height(), rgba.into_raw());
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encoded(format: ImageFormat, width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Cursor::new(Vec::new());
        DynamicImage::new_rgb8(width, height)
            .write_to(&mut bytes, format)
            .unwrap();
        bytes.into_inner()
    }

    #[test]
    fn supported_images_have_proportional_bounded_previews() {
        for (format, mime) in [
            (ImageFormat::Jpeg, "image/jpeg"),
            (ImageFormat::Png, "image/png"),
            (ImageFormat::Gif, "image/gif"),
            (ImageFormat::WebP, "image/webp"),
        ] {
            let bytes = encoded(format, 1600, 400);
            let ImagePreview {
                thumbnail,
                width,
                height,
            } = decode(&bytes, mime).unwrap();
            assert_eq!((width, height), (1600, 400));
            assert_eq!(thumbnail.size(), (128, 32));
            assert_eq!(decode_expanded(&bytes, mime).unwrap().size(), (1280, 320));
        }
    }

    #[test]
    fn oversized_logical_screen_is_rejected_before_allocating_pixels() {
        let mut gif = encoded(ImageFormat::Gif, 1, 1);
        gif[6..8].copy_from_slice(&8192_u16.to_le_bytes());
        gif[8..10].copy_from_slice(&8192_u16.to_le_bytes());
        assert!(decode(&gif, "image/gif").is_err());
        assert!(decode(b"broken", "image/png").is_err());
    }

    #[test]
    fn webp_chunk_lengths_and_inner_frames_cannot_bypass_the_canvas_limits() {
        let mut webp = encoded(ImageFormat::WebP, 3, 2);
        webp[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode(&webp, "image/webp").is_err());
        let mut vp8 = *b"VP8 \x0a\0\0\0\0\0\0\x9d\x01\x2a\0\0\0\0";
        vp8[14..16].copy_from_slice(&8192_u16.to_le_bytes());
        vp8[16..18].copy_from_slice(&8192_u16.to_le_bytes());
        assert!(check_webp_chunks(&vp8, true).is_err());
    }

    #[test]
    fn phone_orientation_applies_to_both_previews_and_reported_dimensions() {
        let mut jpeg = encoded(ImageFormat::Jpeg, 200, 100);
        // APP1 / Exif TIFF with a single orientation tag: rotate 90 degrees.
        let orientation = b"\xff\xe1\x00\x22Exif\0\0II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0";
        jpeg.splice(2..2, orientation.iter().copied());
        let preview = decode(&jpeg, "image/jpeg").unwrap();
        assert_eq!((preview.width, preview.height), (100, 200));
        assert_eq!(
            decode_expanded(&jpeg, "image/jpeg").unwrap().size(),
            (100, 200)
        );
        assert_eq!(preview.thumbnail.size(), (64, 128));
    }
}
