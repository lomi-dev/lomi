use super::{Session, Terminals};
use crate::{files::main_window, shell};
use image::{codecs::png::PngEncoder, ExtendedColorType, ImageEncoder};
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};
use tauri::{AppHandle, Manager, State, Window};
use tauri_plugin_clipboard_manager::ClipboardExt;

const PIXEL_LIMIT: u64 = 32 * 1024 * 1024;
const IMAGE_LIMIT: u64 = 32 * 1024 * 1024;
const CACHE_LIMIT: u64 = 512 * 1024 * 1024;
const FILE_LIMIT: usize = 2048;
// Clipboard decoding and PNG encoding can each hold a full uncompressed image.
static PASTE_LOCK: Mutex<()> = Mutex::new(());

struct LimitedWriter<W> {
    inner: W,
    remaining: u64,
}

impl<W: Write> Write for LimitedWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() as u64 > self.remaining {
            return Err(io::Error::other("Clipboard PNG exceeds the 32 MiB limit."));
        }
        let written = self.inner.write(bytes)?;
        self.remaining -= written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

fn image_directory(cache: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(cache).map_err(|error| error.to_string())?;
    let directory = cache.join("terminal-clipboard");
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(&directory) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.to_string()),
    }
    if !fs::symlink_metadata(&directory)
        .map_err(|error| error.to_string())?
        .is_dir()
    {
        return Err("The terminal clipboard cache must be a regular directory.".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
            .map_err(|error| error.to_string())?;
    }
    fs::canonicalize(directory).map_err(|error| error.to_string())
}

fn validate_image(rgba: &[u8], width: u32, height: u32) -> Result<(), String> {
    let pixels = u64::from(width) * u64::from(height);
    if pixels == 0 || pixels > PIXEL_LIMIT || pixels * 4 != rgba.len() as u64 {
        return Err("Clipboard images must contain at most 32 megapixels of RGBA data.".into());
    }
    Ok(())
}

fn save_image(cache: &Path, rgba: &[u8], width: u32, height: u32) -> Result<PathBuf, String> {
    validate_image(rgba, width, height)?;
    let directory = image_directory(cache)?;
    let mut bytes = 0u64;
    let mut count = 0usize;
    for entry in fs::read_dir(&directory).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        bytes = bytes.saturating_add(
            fs::symlink_metadata(entry.path())
                .map_err(|error| error.to_string())?
                .len(),
        );
        count += 1;
        if bytes > CACHE_LIMIT || count >= FILE_LIMIT {
            return Err(cache_full(&directory));
        }
    }
    let mut file = tempfile::Builder::new()
        .prefix("image-")
        .suffix(".png")
        .tempfile_in(&directory)
        .map_err(|error| error.to_string())?;
    PngEncoder::new(LimitedWriter {
        inner: file.as_file_mut(),
        remaining: IMAGE_LIMIT,
    })
    .write_image(rgba, width, height, ExtendedColorType::Rgba8)
    .map_err(|error| format!("Cannot save clipboard image: {error}"))?;
    if bytes
        + file
            .as_file()
            .metadata()
            .map_err(|error| error.to_string())?
            .len()
        > CACHE_LIMIT
    {
        return Err(cache_full(&directory));
    }
    file.as_file()
        .sync_all()
        .map_err(|error| error.to_string())?;
    // Keep images after terminal close and app restart: CLI drafts may still reference them.
    file.keep()
        .map(|(_, path)| path)
        .map_err(|error| error.to_string())
}

fn cache_full(directory: &Path) -> String {
    format!(
        "Terminal clipboard storage is full (512 MiB or 2048 images). Remove images you no longer need from {} and paste again.",
        directory.display()
    )
}

#[tauri::command]
pub async fn paste_terminal_clipboard(
    window: Window,
    app: AppHandle,
    terminals: State<'_, Terminals>,
    id: String,
) -> Result<Option<String>, String> {
    main_window(&window)?;
    let terminals = terminals.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = PASTE_LOCK.lock().map_err(|error| error.to_string())?;
        let session = terminals.get(&id)?;
        // Image reads must stay off the GUI thread, including on Linux.
        match app.clipboard().read_image() {
            Ok(image) => {
                validate_image(image.rgba(), image.width(), image.height())?;
                if let Some(process) = native_image_paste_process(&session) {
                    // agy does not attach pasted paths. Its existing Ctrl+V action
                    // imports media without changing configuration or submitting input.
                    let mut writer = session.writer.lock().map_err(|error| error.to_string())?;
                    let writer = writer.as_mut().ok_or("The terminal has been closed.")?;
                    if native_image_paste_process(&session) != Some(process) {
                        return Err("The foreground program changed before the image could be pasted. Paste again.".into());
                    }
                    writer.write_all(b"\x16").and_then(|_| writer.flush()).map_err(|error| error.to_string())?;
                    return Ok(None);
                }
                let cache = app
                    .path()
                    .app_cache_dir()
                    .map_err(|error| error.to_string())?;
                let path = save_image(&cache, image.rgba(), image.width(), image.height())?;
                let result = (|| {
                    let path = dunce::simplified(&path).to_string_lossy();
                    let path = if let Some(distro) = &session.profile.distro {
                        shell::wsl_path(distro, &path)?
                    } else {
                        path.into_owned()
                    };
                    shell::quote(&path, &session.profile.kind).map(|quoted| format!("{quoted} "))
                })();
                if result.is_err() {
                    let _ = fs::remove_file(path);
                }
                result.map(Some)
            }
            Err(image_error) => {
                let text = app.clipboard().read_text().map_err(|text_error| {
                    format!(
                        "Cannot paste clipboard contents. Image: {image_error}; text: {text_error}"
                    )
                })?;
                if text.len() > 1024 * 1024 {
                    return Err("Clipboard text exceeds the 1 MiB paste limit.".into());
                }
                Ok(Some(text))
            }
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

fn native_image_paste_process(session: &Session) -> Option<u32> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    if session.profile.distro.is_none() && session.foreground_program().as_deref() == Some("agy") {
        return session
            .master
            .lock()
            .ok()?
            .process_group_leader()
            .map(|pid| pid as u32);
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let _ = session;
    None
}
