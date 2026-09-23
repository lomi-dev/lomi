use serde::Serialize;
use std::{
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    sync::Mutex,
};
use tauri::{Manager, State, Window};

#[cfg(unix)]
pub(crate) mod agent;
#[cfg(unix)]
pub(crate) mod agent_image;
pub mod editor;
pub mod images;
pub mod markdown;
pub mod operations;
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
    if relative.components().any(|part| {
        matches!(
            part,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err("The path must stay inside the project.".into());
    }
    let path = fs::canonicalize(root.join(relative)).map_err(|error| error.to_string())?;
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
        if name == ".git" {
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
        if !path.is_file() {
            return Err("Only regular files can be previewed.".into());
        }
        let mut bytes = Vec::new();
        fs::File::open(path)
            .map_err(|error| error.to_string())?
            .take(1_048_577)
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        if bytes.len() > 1_048_576 {
            return Err("This file is larger than the 1 MiB preview limit.".into());
        }
        if bytes.contains(&0) {
            return Err("Binary files cannot be previewed as text.".into());
        }
        String::from_utf8(bytes).map_err(|_| "This file is not UTF-8 text.".into())
    })
    .await
    .map_err(|error| error.to_string())?
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
pub fn load_session(
    window: Window,
    app: tauri::AppHandle,
    state: State<'_, SessionFile>,
) -> Result<Option<serde_json::Value>, String> {
    main_window(&window)?;
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
}

#[tauri::command]
pub fn save_session(
    window: Window,
    app: tauri::AppHandle,
    state: State<'_, SessionFile>,
    data: serde_json::Value,
    recovery: Option<bool>,
) -> Result<(), String> {
    main_window(&window)?;
    let _guard = state.0.lock().map_err(|error| error.to_string())?;
    let path = session_path(&app)?;
    prepare_session_save(&path, &data, recovery.unwrap_or(false))?;
    write_json(&path, &data, 8 * 1024 * 1024)
}

fn prepare_session_save(
    path: &Path,
    data: &serde_json::Value,
    recovery: bool,
) -> Result<(), String> {
    use std::io::{Read, Write};
    if data["version"] != 3 || !data["projects"].is_array() {
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
        .filter(|v| [1, 2, 3].contains(v));
    let valid = version.is_some() && saved.as_ref().is_some_and(|v| v["projects"].is_array());
    if !valid && !recovery {
        return Err(
            "The saved session is corrupt or unsupported. Choose recovery before replacing it."
                .into(),
        );
    }
    if version == Some(3) && valid && !recovery {
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
    let temporary = path.with_extension("json.tmp");
    {
        use std::io::Write;
        let mut file = fs::File::create(&temporary).map_err(|error| error.to_string())?;
        file.write_all(&bytes).map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
    }
    fs::rename(temporary, path).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn session_upgrade_backs_up_exact_legacy_bytes_and_preserves_unknown_files() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("session.json");
        let output = serde_json::json!({"version":3,"projects":[]});
        for version in [1, 2] {
            let bytes = format!("{{ \"version\": {version}, \"projects\": [] }}\n");
            fs::write(&path, &bytes).unwrap();
            prepare_session_save(&path, &output, false).unwrap();
            assert_eq!(
                fs::read(path.with_file_name(format!("session.v{version}.json"))).unwrap(),
                bytes.as_bytes()
            );
            fs::write(
                &path,
                format!("{{\"version\":{version},\"projects\":[],\"other\":true}}"),
            )
            .unwrap();
            prepare_session_save(&path, &output, false).unwrap();
            assert_eq!(
                fs::read(path.with_file_name(format!("session.v{version}.json"))).unwrap(),
                bytes.as_bytes()
            );
        }
        for bytes in ["{broken", "{\"version\":99,\"projects\":[]}"] {
            fs::write(&path, bytes).unwrap();
            assert!(prepare_session_save(&path, &output, false).is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), bytes);
            prepare_session_save(&path, &output, true).unwrap();
        }
    }
    #[test]
    fn rejects_parent_paths_and_lists_directories_first() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("a.txt"), "test").unwrap();
        fs::create_dir(root.path().join("z-folder")).unwrap();
        let root = root.path().to_str().unwrap();
        assert!(inside(root, "../").is_err());
        let entries = read_directory(root, "").unwrap();
        assert_eq!(entries[0].name, "z-folder");
        assert_eq!(entries[1].name, "a.txt");
    }
    #[cfg(unix)]
    #[test]
    fn refuses_symlinks_outside_the_project() {
        let root = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink("/", root.path().join("outside")).unwrap();
        assert!(inside(root.path().to_str().unwrap(), "outside").is_err());
    }
}
