//! Workspace logos: project favicon discovery and user-picked logo files.

use base64::{engine::general_purpose::STANDARD, Engine};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};
use tauri::{Manager, Window};

const MAX_LOGO_BYTES: u64 = 512 * 1024;
const LOGO_EXTENSIONS: [&str; 5] = ["png", "jpg", "svg", "webp", "ico"];
const FAVICON_DIRS: [&str; 7] = ["", "public", "static", "assets", "app", "src/app", "src"];
const FAVICON_NAMES: [&str; 3] = ["favicon.ico", "favicon.png", "favicon.svg"];

fn mime(extension: &str) -> &'static str {
    match extension {
        "png" => "image/png",
        "jpg" => "image/jpeg",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        _ => "image/x-icon",
    }
}

fn data_url(extension: &str, bytes: &[u8]) -> String {
    format!("data:{};base64,{}", mime(extension), STANDARD.encode(bytes))
}

/// Returns the first favicon that is a regular file inside the project.
fn find_favicon(root: &Path) -> Option<String> {
    let root = fs::canonicalize(root).ok()?;
    for directory in FAVICON_DIRS {
        for name in FAVICON_NAMES {
            // Canonicalizing resolves symlinks so escapes fail the prefix check.
            let Ok(path) = fs::canonicalize(root.join(directory).join(name)) else {
                continue;
            };
            let Ok(metadata) = fs::metadata(&path) else {
                continue;
            };
            if !path.starts_with(&root) || !metadata.is_file() || metadata.len() > MAX_LOGO_BYTES {
                continue;
            }
            let Ok(bytes) = fs::read(&path) else {
                continue;
            };
            return Some(data_url(name.rsplit('.').next()?, &bytes));
        }
    }
    None
}

fn valid_logo_id(id: &str) -> Option<&str> {
    let (stem, extension) = id.split_once('.')?;
    (stem.len() == 32
        && stem
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        && LOGO_EXTENSIONS.contains(&extension))
    .then_some(extension)
}

fn logo_directory(window: &Window) -> Result<PathBuf, String> {
    Ok(window
        .app_handle()
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?
        .join("workspace-logos"))
}

#[tauri::command]
pub async fn workspace_favicon(window: Window, project: String) -> Result<Option<String>, String> {
    crate::files::main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        Ok(find_favicon(&crate::files::directory(&project)?))
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Origin remote URL for the frontend to parse; None outside a Git repository.
#[tauri::command]
pub async fn workspace_origin_url(
    window: Window,
    project: String,
) -> Result<Option<String>, String> {
    crate::files::main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        let root = crate::files::directory(&project)?;
        Ok(crate::git::checked(&root, &["remote", "get-url", "origin"])
            .ok()
            .map(|output| String::from_utf8_lossy(&output).trim().to_owned())
            .filter(|url| !url.is_empty()))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn workspace_logo_pick(window: Window) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    crate::files::main_window(&window)?;
    let directory = logo_directory(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        let Some(selected) = window
            .app_handle()
            .dialog()
            .file()
            .set_parent(&window)
            .set_title("Choose workspace logo")
            .add_filter("Image", &["png", "jpg", "jpeg", "svg", "webp", "ico"])
            .blocking_pick_file()
        else {
            return Ok(None);
        };
        let path = selected
            .into_path()
            .map_err(|_| "Select a local image file.".to_string())?;
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .map(str::to_ascii_lowercase)
            .map(|value| if value == "jpeg" { "jpg".into() } else { value })
            .filter(|value| LOGO_EXTENSIONS.contains(&value.as_str()))
            .ok_or("Choose a PNG, JPG, SVG, WebP, or ICO image.")?;
        let metadata = fs::metadata(&path).map_err(|error| error.to_string())?;
        if !metadata.is_file() || metadata.len() > MAX_LOGO_BYTES {
            return Err("Choose an image up to 512 KB.".to_string());
        }
        let bytes = fs::read(&path).map_err(|error| error.to_string())?;
        // Content-addressed id: stable, collision-free and not path-derived.
        let digest = Sha256::digest(&bytes);
        let id = format!(
            "{}.{extension}",
            digest[..16]
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        fs::write(directory.join(&id), &bytes).map_err(|error| error.to_string())?;
        Ok(Some(id))
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Logo files stay on disk when a workspace is deleted; they are small and deduplicated.
#[tauri::command]
pub async fn workspace_logo_read(window: Window, id: String) -> Result<Option<String>, String> {
    crate::files::main_window(&window)?;
    let Some(extension) = valid_logo_id(&id) else {
        return Err("Invalid workspace logo.".into());
    };
    let path = logo_directory(&window)?.join(&id);
    let extension = extension.to_owned();
    tauri::async_runtime::spawn_blocking(move || match fs::read(path) {
        Ok(bytes) if bytes.len() as u64 <= MAX_LOGO_BYTES => Ok(Some(data_url(&extension, &bytes))),
        _ => Ok(None),
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_favicon_in_priority_order_and_limits_size() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("public")).unwrap();
        fs::write(dir.path().join("public/favicon.svg"), "<svg/>").unwrap();
        assert!(find_favicon(dir.path())
            .unwrap()
            .starts_with("data:image/svg+xml;base64,"));
        fs::write(dir.path().join("favicon.png"), [1u8]).unwrap();
        assert!(find_favicon(dir.path())
            .unwrap()
            .starts_with("data:image/png;base64,"));
        fs::write(
            dir.path().join("favicon.png"),
            vec![0u8; MAX_LOGO_BYTES as usize + 1],
        )
        .unwrap();
        assert!(find_favicon(dir.path())
            .unwrap()
            .starts_with("data:image/svg+xml"));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_outside_project() {
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret.png"), [1u8]).unwrap();
        let project = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(
            outside.path().join("secret.png"),
            project.path().join("favicon.png"),
        )
        .unwrap();
        assert_eq!(find_favicon(project.path()), None);
    }

    #[test]
    fn validates_logo_ids() {
        assert_eq!(
            valid_logo_id(&format!("{}.png", "a".repeat(32))),
            Some("png")
        );
        for id in ["../x.png", "abc.png", &format!("{}.gif", "a".repeat(32))] {
            assert_eq!(valid_logo_id(id), None);
        }
    }
}
