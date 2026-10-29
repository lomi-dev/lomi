use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits};
use std::{
    io::{Cursor, Read},
    sync::Mutex,
};
use tauri::{ipc::Response, Window};

const FILE_LIMIT: u64 = 32 * 1024 * 1024;
const PIXEL_LIMIT: u64 = 16 * 1024 * 1024;
// Serial decoding bounds concurrent allocations when several panels open at once.
pub(super) static READ_LOCK: Mutex<()> = Mutex::new(());

fn converted_format(extension: &str) -> Result<Option<ImageFormat>, String> {
    Ok(match extension {
        "png" | "apng" | "jpg" | "jpeg" | "jpe" | "jfif" | "webp" | "gif" | "svg" | "avif"
        | "ico" | "bmp" | "dib" => None,
        "tif" | "tiff" => Some(ImageFormat::Tiff),
        "tga" => Some(ImageFormat::Tga),
        "dds" => Some(ImageFormat::Dds),
        "pbm" | "pgm" | "ppm" | "pam" | "pnm" => Some(ImageFormat::Pnm),
        "qoi" => Some(ImageFormat::Qoi),
        "hdr" => Some(ImageFormat::Hdr),
        "exr" => Some(ImageFormat::OpenExr),
        "ff" => Some(ImageFormat::Farbfeld),
        _ => return Err("This image format is not supported.".into()),
    })
}

fn convert(bytes: Vec<u8>, format: ImageFormat) -> Result<Vec<u8>, String> {
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(|error| error.to_string())?;
    let (width, height) = decoder.dimensions();
    if u64::from(width) * u64::from(height) > PIXEL_LIMIT {
        return Err("This image exceeds the 16 megapixel conversion limit.".into());
    }
    if decoder.total_bytes() > 128 * 1024 * 1024 {
        return Err("This image exceeds the 128 MiB decoded image limit.".into());
    }
    let orientation = decoder.orientation().map_err(|error| error.to_string())?;
    let mut decoded = DynamicImage::from_decoder(decoder).map_err(|error| error.to_string())?;
    decoded.apply_orientation(orientation);
    let mut output = Cursor::new(Vec::new());
    // Webviews receive an ordinary image, never an executable document or a file URL.
    DynamicImage::ImageRgba8(decoded.into_rgba8())
        .write_to(&mut output, ImageFormat::Png)
        .map_err(|error| error.to_string())?;
    Ok(output.into_inner())
}

fn read(root: &str, relative: &str) -> Result<Vec<u8>, String> {
    let _guard = READ_LOCK.lock().map_err(|error| error.to_string())?;
    let path = super::inside(root, relative)?;
    read_resolved(&path)
}

pub(super) fn read_resolved(path: &std::path::Path) -> Result<Vec<u8>, String> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let format = converted_format(&extension)?;
    let file = super::resolved::open_resolved_file(path).map_err(|error| error.to_string())?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("Only regular image files can be opened.".into());
    }
    if metadata.len() > FILE_LIMIT {
        return Err("This image exceeds the 32 MiB preview limit.".into());
    }
    let mut bytes = Vec::new();
    file.take(FILE_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > FILE_LIMIT {
        return Err("This image exceeds the 32 MiB preview limit.".into());
    }
    if let Some(format) = format {
        convert(bytes, format).map_err(|error| format!("Cannot decode this image: {error}"))
    } else {
        Ok(bytes)
    }
}

#[tauri::command]
pub async fn read_image_file(
    window: Window,
    root: String,
    relative: String,
) -> Result<Response, String> {
    super::main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || read(&root, &relative).map(Response::new))
        .await
        .map_err(|error| error.to_string())?
}
