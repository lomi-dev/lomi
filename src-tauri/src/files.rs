use serde::Serialize;
use std::{
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    sync::Mutex,
};
use tauri::{Manager, Window};

#[cfg(unix)]
pub(crate) mod agent;
#[cfg(unix)]
pub(crate) mod agent_image;
pub mod editor;
pub mod images;
pub mod markdown;
pub mod operations;
pub(crate) mod resolved;
pub mod search;
pub mod watch;

#[derive(Default)]
pub struct SessionFile(pub Mutex<()>);

pub fn main_window(window: &Window) -> Result<(), String> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err("This operation is only available in the main window.".into())
    }
}

pub fn directory(path: &str) -> Result<PathBuf, String> {
    let path = fs::canonicalize(path).map_err(|error| format!("Cannot open directory: {error}"))?;
    if !path.is_dir() {
        return Err("The selected path is not a directory.".into());
    }
    Ok(path)
}

pub fn inside(root: &str, relative: &str) -> Result<PathBuf, String> {
    let root = directory(root)?;
    let relative = Path::new(relative);
    let private = |path: &Path| {
        path.components().any(|part| {
            matches!(part, Component::Normal(name)
                if lomi_files::is_private_staging_name(name.as_encoded_bytes()))
        })
    };
    if private(&root) || private(relative) {
        return Err("This path is reserved for private file recovery.".into());
    }
    if relative.components().any(|part| {
        matches!(
            part,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err("The path must stay inside the project.".into());
    }
    let path = fs::canonicalize(root.join(relative)).map_err(|error| error.to_string())?;
    if private(&path) {
        return Err("This path is reserved for private file recovery.".into());
    }
    if !path.starts_with(&root) {
        return Err("This symbolic link points outside the project.".into());
    }
    Ok(path)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    name: String,
    relative_path: String,
    path: String,
    is_directory: bool,
    is_symlink: bool,
}

pub fn read_directory(root: &str, relative: &str) -> Result<Vec<Entry>, String> {
    let path = inside(root, relative)?;
    let mut entries = Vec::new();
    for entry in fs::read_dir(path).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == ".git" || lomi_files::is_private_staging_name(name.as_bytes()) {
            continue;
        }
        let kind = entry.file_type().map_err(|error| error.to_string())?;
        entries.push(Entry {
            relative_path: Path::new(relative)
                .join(&name)
                .to_string_lossy()
                .into_owned(),
            name,
            path: entry.path().to_string_lossy().into_owned(),
            is_directory: entry.path().is_dir(),
            is_symlink: kind.is_symlink(),
        });
    }
    entries.sort_by(|a, b| {
        b.is_directory
            .cmp(&a.is_directory)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(entries)
}

#[tauri::command]
pub async fn list_directory(
    window: Window,
    root: String,
    relative: String,
) -> Result<Vec<Entry>, String> {
    main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || read_directory(&root, &relative))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn validate_directory(window: Window, path: String) -> Result<String, String> {
    main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        directory(&path).map(|path| path.to_string_lossy().into_owned())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn preview_file(
    window: Window,
    root: String,
    relative: String,
) -> Result<String, String> {
    main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        let path = inside(&root, &relative)?;
        preview_resolved(&path)
    })
    .await
    .map_err(|error| error.to_string())?
}

fn preview_resolved(path: &Path) -> Result<String, String> {
    let file = resolved::open_resolved_file(path).map_err(|error| error.to_string())?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("Only regular files can be previewed.".into());
    }
    if metadata.len() > 1_048_576 {
        return Err("This file is larger than the 1 MiB preview limit.".into());
    }
    let mut bytes = Vec::new();
    file.take(1_048_577)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > 1_048_576 {
        return Err("This file is larger than the 1 MiB preview limit.".into());
    }
    if bytes.contains(&0) {
        return Err("Binary files cannot be previewed as text.".into());
    }
    String::from_utf8(bytes).map_err(|_| "This file is not UTF-8 text.".into())
}

fn session_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let path = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    fs::create_dir_all(&path).map_err(|error| error.to_string())?;
    Ok(path.join("session.json"))
}

#[tauri::command]
pub async fn load_session(
    window: Window,
    app: tauri::AppHandle,
) -> Result<Option<serde_json::Value>, String> {
    main_window(&window)?;
    // Session reads and lock waits must not block the native event loop.
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<SessionFile>();
        let _guard = state.0.lock().map_err(|error| error.to_string())?;
        let path = session_path(&app)?;
        if !path.exists() {
            return Ok(None);
        }
        let mut data = Vec::new();
        fs::File::open(&path)
            .map_err(|error| error.to_string())?
            .take(8 * 1024 * 1024 + 1)
            .read_to_end(&mut data)
            .map_err(|error| error.to_string())?;
        if data.len() > 8 * 1024 * 1024 {
            return Err(format!(
                "The session exceeds 8 MiB. The file has been left intact at {}.",
                path.display()
            ));
        }
        let saved: serde_json::Value = serde_json::from_slice(&data).map_err(|error| {
            format!(
                "Cannot restore the saved session ({error}). The file has been left intact at {}.",
                path.display()
            )
        })?;
        if saved.is_null() {
            return Err(
                "The saved session is null, not an empty layout. Choose recovery before replacing it."
                    .into(),
            );
        }
        Ok(Some(saved))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn save_session(
    window: Window,
    app: tauri::AppHandle,
    data: serde_json::Value,
    recovery: Option<bool>,
) -> Result<(), String> {
    main_window(&window)?;
    // Backup and atomic publication include fsync; retain the session lock
    // throughout the operation without holding the native event loop.
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<SessionFile>();
        let _guard = state.0.lock().map_err(|error| error.to_string())?;
        let path = session_path(&app)?;
        prepare_session_save(&path, &data, recovery.unwrap_or(false))?;
        write_json(&path, &data, 8 * 1024 * 1024)
    })
    .await
    .map_err(|error| error.to_string())?
}

fn prepare_session_save(
    path: &Path,
    data: &serde_json::Value,
    recovery: bool,
) -> Result<(), String> {
    use std::io::{Read, Write};
    if data["version"] != 5 || !data["projects"].is_array() {
        return Err("Unsupported session output.".into());
    }
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.to_string()),
    };
    let mut previous = Vec::new();
    file.take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut previous)
        .map_err(|e| e.to_string())?;
    if previous.len() > 8 * 1024 * 1024 {
        return Err("The previous session exceeds its size limit and was preserved.".into());
    }
    let saved = serde_json::from_slice::<serde_json::Value>(&previous).ok();
    let version = saved
        .as_ref()
        .and_then(|v| v["version"].as_u64())
        .filter(|v| [1, 2, 3, 4, 5].contains(v));
    let valid = version.is_some() && saved.as_ref().is_some_and(|v| v["projects"].is_array());
    if !valid && !recovery {
        return Err(
            "The saved session is corrupt or unsupported. Choose recovery before replacing it."
                .into(),
        );
    }
    if version == Some(5) && valid && !recovery {
        return Ok(());
    }
    let name = if recovery {
        format!(
            "session.recovery.{}.json",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        )
    } else {
        format!("session.v{}.json", version.unwrap())
    };
    let mut backup = tempfile::NamedTempFile::new_in(path.parent().ok_or("Invalid session path.")?)
        .map_err(|e| e.to_string())?;
    backup.write_all(&previous).map_err(|e| e.to_string())?;
    backup.as_file().sync_all().map_err(|e| e.to_string())?;
    let destination = path.with_file_name(name);
    match backup.persist_noclobber(&destination) {
        Ok(_) => (),
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
            let existing = fs::symlink_metadata(&destination).map_err(|e| e.to_string())?;
            if !existing.is_file() || existing.file_type().is_symlink() {
                return Err("The session backup path is not a regular file. The original session was preserved.".into());
            }
        }
        Err(error) => return Err(error.to_string()),
    }
    #[cfg(unix)]
    fs::File::open(path.parent().unwrap())
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn write_json(path: &Path, data: &impl Serialize, limit: usize) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(data).map_err(|error| error.to_string())?;
    if bytes.len() > limit {
        return Err(format!(
            "The settings file exceeds its {} KiB limit.",
            limit / 1024
        ));
    }
    #[cfg(windows)]
    let _write_guard = {
        // Serialize ACL updates and atomic replacements for app-owned settings.
        static JSON_WRITES: Mutex<()> = Mutex::new(());
        JSON_WRITES
            .lock()
            .map_err(|_| "Cannot serialize application settings updates.")?
    };
    use crate::chat::storage::{private, reject_link};
    use std::io::Write;
    let parent = path.parent().ok_or("Invalid settings path.")?;
    reject_link(parent)?;
    reject_link(path)?;
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if !metadata.is_file() {
            return Err("The settings destination must be a regular file.".into());
        }
    }
    // These callers persist app-owned settings, never arbitrary project files.
    private(parent, true)?;
    let parent = fs::canonicalize(parent).map_err(|error| error.to_string())?;
    let destination = parent.join(path.file_name().ok_or("Invalid settings filename.")?);
    let mut temporary =
        tempfile::NamedTempFile::new_in(&parent).map_err(|error| error.to_string())?;
    private(temporary.path(), false)?;
    temporary
        .write_all(&bytes)
        .map_err(|error| error.to_string())?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| error.to_string())?;
    reject_link(&destination)?;
    #[cfg(windows)]
    {
        // Windows can briefly deny replacement while a concurrent reader
        // closes. Retry the same complete file within a short deadline;
        // persistent locks still return an error and preserve the destination.
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(100);
        loop {
            match temporary.persist(&destination) {
                Ok(_) => break,
                Err(error)
                    if matches!(error.error.raw_os_error(), Some(5 | 32))
                        && std::time::Instant::now() < deadline =>
                {
                    temporary = error.file;
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
                Err(error) => return Err(error.to_string()),
            }
        }
    }
    #[cfg(not(windows))]
    temporary
        .persist(&destination)
        .map_err(|error| error.to_string())?;
    #[cfg(unix)]
    fs::File::open(parent)
        .and_then(|file| file.sync_all())
        .map_err(|error| error.to_string())?;
    Ok(())
}
