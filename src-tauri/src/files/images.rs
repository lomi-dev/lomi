use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits};
use std::{
    fs,
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
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let format = converted_format(&extension)?;
    let metadata = fs::metadata(&path).map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("Only regular image files can be opened.".into());
    }
    if metadata.len() > FILE_LIMIT {
        return Err("This image exceeds the 32 MiB preview limit.".into());
    }
    let file = fs::File::open(path).map_err(|error| error.to_string())?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_images_without_changing_their_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().to_str().unwrap();
        for extension in [
            "PNG", "apng", "jpg", "jpeg", "jpe", "jfif", "webp", "gif", "svg", "avif", "ico",
            "bmp", "dib",
        ] {
            let name = format!("image.{extension}");
            let bytes = b"original image bytes";
            fs::write(directory.path().join(&name), bytes).unwrap();
            assert_eq!(read(root, &name).unwrap(), bytes);
        }
    }

    #[test]
    fn converts_non_web_formats_to_png_without_writing_to_disk() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().to_str().unwrap();
        let original = DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            3,
            2,
            image::Rgba([24, 80, 120, 128]),
        ));
        for (extension, format) in [
            ("tiff", ImageFormat::Tiff),
            ("tga", ImageFormat::Tga),
            ("qoi", ImageFormat::Qoi),
            ("ff", ImageFormat::Farbfeld),
        ] {
            let path = directory.path().join(format!("image.{extension}"));
            if format == ImageFormat::Farbfeld {
                DynamicImage::ImageRgba16(original.to_rgba16())
                    .save_with_format(&path, format)
                    .unwrap();
            } else {
                original.save_with_format(&path, format).unwrap();
            }
            let before = fs::read(&path).unwrap();
            let converted = read(root, path.file_name().unwrap().to_str().unwrap()).unwrap();
            let decoded =
                image::load_from_memory_with_format(&converted, ImageFormat::Png).unwrap();
            assert_eq!(decoded.to_rgba8(), original.to_rgba8());
            assert_eq!(fs::read(path).unwrap(), before);
        }
        fs::write(
            directory.path().join("image.ppm"),
            b"P3\n2 1\n255\n255 0 0 0 255 0\n",
        )
        .unwrap();
        let png = read(root, "image.ppm").unwrap();
        let decoded = image::load_from_memory_with_format(&png, ImageFormat::Png).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (2, 1));
    }

    #[test]
    fn rejects_invalid_outside_and_oversized_files() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().to_str().unwrap();
        fs::write(directory.path().join("text.txt"), "text").unwrap();
        fs::write(directory.path().join("broken.tiff"), "not a TIFF").unwrap();
        fs::create_dir(directory.path().join("folder.png")).unwrap();
        fs::File::create(directory.path().join("large.png"))
            .unwrap()
            .set_len(FILE_LIMIT + 1)
            .unwrap();
        for name in [
            "text.txt",
            "broken.tiff",
            "folder.png",
            "large.png",
            "missing.png",
            "../image.png",
        ] {
            assert!(read(root, name).is_err(), "{name}");
        }
        assert!(read(root, directory.path().join("large.png").to_str().unwrap()).is_err());
        assert!(convert(b"P3\n16384 16384\n255\n".to_vec(), ImageFormat::Pnm).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinks_outside_the_project() {
        let directory = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let path = outside.path().join("private.png");
        fs::write(&path, b"private").unwrap();
        std::os::unix::fs::symlink(path, directory.path().join("linked.png")).unwrap();
        assert!(read(directory.path().to_str().unwrap(), "linked.png").is_err());
    }
}
