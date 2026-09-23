//! Bounded, static preview derivatives of already authorized file bytes.
use base64::Engine;
use image::{
    DynamicImage, GenericImageView, ImageDecoder, ImageEncoder, ImageFormat, ImageReader, Limits,
};
use lomi_control_protocol::editor::{EditorFileKind, PreparedEditorBody, PreparedPreviewImage};
use lomi_control_protocol::ErrorCode;
use std::io::{self, Cursor, Write};

pub(crate) fn prepare_editor(
    relative: &str,
    bytes: &[u8],
) -> Result<PreparedEditorBody, ErrorCode> {
    if EditorFileKind::from_relative(relative) == EditorFileKind::Image {
        Ok(PreparedEditorBody::Image(prepare_asset(relative, bytes)?))
    } else {
        let (content, encoding) = super::agent::decode_full(bytes)?;
        Ok(PreparedEditorBody::Text { content, encoding })
    }
}
pub(crate) fn prepare_asset(
    relative: &str,
    bytes: &[u8],
) -> Result<PreparedPreviewImage, ErrorCode> {
    let kind = EditorFileKind::from_relative(relative);
    if kind == EditorFileKind::Svg {
        if bytes.len() > 1024 * 1024 {
            return Err(ErrorCode::ResourceExhausted);
        }
        let (text, _) = super::agent::decode_full(bytes)?;
        return Ok(PreparedPreviewImage {
            data_base64: base64::engine::general_purpose::STANDARD.encode(text),
            mime_type: "image/svg+xml".into(),
            width: 0,
            height: 0,
            original_width: 0,
            original_height: 0,
        });
    }
    if kind != EditorFileKind::Image {
        return Err(ErrorCode::UnsupportedCapability);
    }
    let image = prepare(bytes, relative)?;
    Ok(PreparedPreviewImage {
        data_base64: base64::engine::general_purpose::STANDARD.encode(image.png),
        mime_type: "image/png".into(),
        width: image.width,
        height: image.height,
        original_width: image.original_width,
        original_height: image.original_height,
    })
}

const INPUT_BYTES: usize = 4 * 1024 * 1024;
const OUTPUT_BYTES: usize = 3 * 1024 * 1024;
const PIXELS: u64 = 16 * 1024 * 1024;
const DECODED_BYTES: u64 = 64 * 1024 * 1024;

pub(crate) struct PreviewImage {
    pub png: Vec<u8>,
    pub original_width: u32,
    pub original_height: u32,
    pub width: u32,
    pub height: u32,
}
struct Output(Vec<u8>);
impl Write for Output {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > OUTPUT_BYTES.saturating_sub(self.0.len()) {
            return Err(io::Error::other("Preview exceeds the output budget"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn decode_error(error: image::ImageError) -> ErrorCode {
    if matches!(error, image::ImageError::Limits(_)) {
        ErrorCode::ResourceExhausted
    } else {
        ErrorCode::UnsupportedCapability
    }
}

pub(crate) fn prepare(bytes: &[u8], relative: &str) -> Result<PreviewImage, ErrorCode> {
    let _guard = super::images::READ_LOCK
        .try_lock()
        .map_err(|_| ErrorCode::TargetBusy)?;
    decode(bytes, relative)
}
fn decode(bytes: &[u8], relative: &str) -> Result<PreviewImage, ErrorCode> {
    if bytes.len() > INPUT_BYTES {
        return Err(ErrorCode::ResourceExhausted);
    }
    let format = image::guess_format(bytes)
        .ok()
        .or_else(|| {
            std::path::Path::new(relative)
                .extension()
                .and_then(|v| v.to_str())
                .and_then(ImageFormat::from_extension)
        })
        .ok_or(ErrorCode::UnsupportedCapability)?;
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(DECODED_BYTES);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(decode_error)?;
    let (width, height) = decoder.dimensions();
    // Decoder allocation limits are best-effort; independently bound the output
    // allocation before DynamicImage::from_decoder, which does not check them.
    if width == 0
        || height == 0
        || u64::from(width) * u64::from(height) > PIXELS
        || decoder.total_bytes() > DECODED_BYTES
    {
        return Err(ErrorCode::ResourceExhausted);
    }
    let orientation = decoder.orientation().map_err(decode_error)?;
    let mut decoded = DynamicImage::from_decoder(decoder).map_err(decode_error)?;
    decoded.apply_orientation(orientation);
    let (original_width, original_height) = decoded.dimensions();
    let image = if original_width > 1280 || original_height > 1280 {
        decoded.thumbnail(1280, 1280)
    } else {
        decoded
    }
    .into_rgba8();
    let (width, height) = image.dimensions();
    let mut output = Output(Vec::new());
    image::codecs::png::PngEncoder::new(&mut output)
        .write_image(
            image.as_raw(),
            width,
            height,
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|_| ErrorCode::ResourceExhausted)?;
    Ok(PreviewImage {
        png: output.0,
        original_width,
        original_height,
        width,
        height,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    #[test]
    fn preview_preserves_alpha_and_uses_only_the_first_animation_frame() {
        let image = RgbaImage::from_pixel(2, 3, Rgba([22, 70, 90, 100]));
        let mut source = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(image.clone())
            .write_to(&mut source, ImageFormat::Png)
            .unwrap();
        let preview = decode(source.get_ref(), "image.png").unwrap();
        assert_eq!(
            (
                preview.original_width,
                preview.original_height,
                preview.width,
                preview.height
            ),
            (2, 3, 2, 3)
        );
        let decoded = image::load_from_memory(&preview.png).unwrap().into_rgba8();
        assert_eq!(decoded, image);

        let mut animated = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut animated);
            encoder
                .encode_frame(image::Frame::new(RgbaImage::from_pixel(
                    2,
                    3,
                    Rgba([255, 0, 0, 255]),
                )))
                .unwrap();
            encoder
                .encode_frame(image::Frame::new(RgbaImage::from_pixel(
                    2,
                    3,
                    Rgba([0, 0, 255, 255]),
                )))
                .unwrap();
        }
        let preview = decode(&animated, "animation.gif").unwrap();
        let decoded = image::load_from_memory(&preview.png).unwrap().into_rgba8();
        assert_eq!(decoded.get_pixel(0, 0), &Rgba([255, 0, 0, 255]));
    }

    #[test]
    fn preview_applies_exif_orientation_and_downscales_without_upscaling() {
        let large = RgbaImage::from_pixel(2560, 1280, Rgba([40, 90, 170, 180]));
        let mut source = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(large)
            .write_to(&mut source, ImageFormat::Png)
            .unwrap();
        let preview = decode(source.get_ref(), "large.png").unwrap();
        assert_eq!(
            (
                preview.original_width,
                preview.original_height,
                preview.width,
                preview.height
            ),
            (2560, 1280, 1280, 640)
        );
        assert!(preview.png.len() <= OUTPUT_BYTES);
        let mut jpeg = Cursor::new(Vec::new());
        DynamicImage::ImageRgb8(image::RgbImage::from_pixel(3, 2, image::Rgb([90, 70, 40])))
            .write_to(&mut jpeg, ImageFormat::Jpeg)
            .unwrap();
        // APP1 EXIF, little-endian TIFF, one SHORT orientation tag (6 = rotate90).
        let exif = b"Exif\0\0II\x2a\x00\x08\x00\x00\x00\x01\x00\x12\x01\x03\x00\x01\x00\x00\x00\x06\x00\x00\x00\x00\x00\x00\x00";
        let mut oriented = vec![0xff, 0xd8, 0xff, 0xe1];
        oriented.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
        oriented.extend_from_slice(exif);
        oriented.extend_from_slice(&jpeg.get_ref()[2..]);
        let preview = decode(&oriented, "camera.jpg").unwrap();
        assert_eq!(
            (
                preview.original_width,
                preview.original_height,
                preview.width,
                preview.height
            ),
            (2, 3, 2, 3)
        );
    }

    #[test]
    fn preview_rejects_declared_pixel_bombs_oversize_inputs_and_non_images() {
        // Complete BMP metadata, with dimensions far beyond the supplied pixels.
        let mut bmp = vec![0_u8; 58];
        bmp[0..2].copy_from_slice(b"BM");
        bmp[2..6].copy_from_slice(&58_u32.to_le_bytes());
        bmp[10..14].copy_from_slice(&54_u32.to_le_bytes());
        bmp[14..18].copy_from_slice(&40_u32.to_le_bytes());
        bmp[18..22].copy_from_slice(&4097_u32.to_le_bytes());
        bmp[22..26].copy_from_slice(&4097_u32.to_le_bytes());
        bmp[26..28].copy_from_slice(&1_u16.to_le_bytes());
        bmp[28..30].copy_from_slice(&24_u16.to_le_bytes());
        assert!(matches!(
            decode(&bmp, "bomb.bmp"),
            Err(ErrorCode::ResourceExhausted)
        ));
        assert!(matches!(
            decode(&vec![0; INPUT_BYTES + 1], "huge.png"),
            Err(ErrorCode::ResourceExhausted)
        ));
        assert!(matches!(
            decode(b"<script>bad()</script>", "fake.png"),
            Err(ErrorCode::UnsupportedCapability)
        ));
    }
}
