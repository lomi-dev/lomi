use std::io::Read;
use tauri::{ipc::Response, Window};

const IMAGE_LIMIT: u64 = 16 * 1024 * 1024;

fn read_image(root: &str, relative: &str) -> Result<Vec<u8>, String> {
    let path = super::inside(root, relative)?;
    read_resolved(&path)
}

pub(super) fn read_resolved(path: &std::path::Path) -> Result<Vec<u8>, String> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !matches!(
        extension.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "avif" | "ico"
    ) {
        return Err("Only image files can be shown in Markdown previews.".into());
    }
    let file = super::resolved::open_resolved_file(path).map_err(|error| error.to_string())?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.len() > IMAGE_LIMIT {
        return Err("Markdown images must be regular files no larger than 16 MiB.".into());
    }
    let mut bytes = Vec::new();
    file.take(IMAGE_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > IMAGE_LIMIT {
        return Err("This image exceeds the 16 MiB preview limit.".into());
    }
    Ok(bytes)
}

#[tauri::command]
pub async fn read_markdown_image(
    window: Window,
    root: String,
    relative: String,
) -> Result<Response, String> {
    super::main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || read_image(&root, &relative).map(Response::new))
        .await
        .map_err(|error| error.to_string())?
}
