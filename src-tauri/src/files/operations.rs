use super::{directory, inside, main_window};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
};
use tauri::{Manager, Window};
use tauri_plugin_opener::OpenerExt;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Operation {
    NewFile {
        name: String,
    },
    NewFolder {
        name: String,
    },
    Rename {
        name: String,
    },
    Copy {
        #[serde(rename = "sourceRoot")]
        source_root: String,
        source: String,
    },
    Move {
        #[serde(rename = "sourceRoot")]
        source_root: String,
        source: String,
    },
    Duplicate,
    Trash,
    Delete,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    pub old_path: Option<String>,
    pub new_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

pub(crate) fn validate_relative(relative: &str) -> Result<(), String> {
    if Path::new(relative).components().any(|part| {
        !matches!(part, Component::Normal(_) | Component::CurDir)
            || part
                .as_os_str()
                .as_encoded_bytes()
                .eq_ignore_ascii_case(b".git")
    }) {
        return Err("The path must stay inside the project and outside .git.".into());
    }
    Ok(())
}

// Resolve the parent, retaining the leaf so rename/delete operate on a symlink itself.
pub(crate) fn entry_path(root: &str, relative: &str) -> Result<PathBuf, String> {
    validate_relative(relative)?;
    if relative.is_empty() {
        return directory(root);
    }
    let relative = Path::new(relative);
    let parent = relative
        .parent()
        .and_then(Path::to_str)
        .ok_or("Invalid parent path.")?;
    let leaf = relative.file_name().ok_or("Choose a file or folder.")?;
    let path = inside(root, parent)?.join(leaf);
    fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
    Ok(path)
}

#[tauri::command]
pub async fn resolve_project_entry(
    window: Window,
    root: String,
    relative: String,
) -> Result<String, String> {
    main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        entry_path(&root, &relative).map(|path| path.to_string_lossy().into_owned())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn file_operation(
    window: Window,
    webview: tauri::Webview,
    app: tauri::AppHandle,
    root: String,
    relative: String,
    operation: Operation,
    expected_path: Option<String>,
) -> Result<FileChange, String> {
    main_window(&window)?;
    crate::shared_host::trusted_full_window(&window, &webview)?;
    let deadline = std::time::Instant::now()
        .checked_add(std::time::Duration::from_secs(120))
        .ok_or("File operation deadline is unavailable.")?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = window.state::<super::editor::EditorFiles>();
        let _lock = state
            .writes
            .lock()
            .map_err(|_| "File writes are unavailable.")?;
        crate::desktop_file_effects::file_operation(
            &app,
            &window,
            &webview,
            &root,
            &relative,
            operation,
            expected_path,
            deadline,
        )
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn open_project_item(
    window: Window,
    root: String,
    relative: String,
    reveal: bool,
) -> Result<(), String> {
    main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        let path = inside(&root, &relative)?;
        if reveal {
            window.opener().reveal_item_in_dir(path)
        } else {
            window
                .opener()
                .open_path(path.to_string_lossy(), None::<&str>)
        }
        .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn git_pull(
    window: Window,
    webview: tauri::Webview,
    app: tauri::AppHandle,
    host: tauri::State<'_, super::super::shared_host::SharedHost>,
    root: String,
    rebase: bool,
) -> Result<(), String> {
    main_window(&window)?;
    crate::shared_host::trusted_full_window(&window, &webview)?;
    let host = host.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = window.state::<super::editor::EditorFiles>();
        // Hold the editor write lock through pull to serialize worktree mutations.
        let _lock = state
            .writes
            .lock()
            .map_err(|_| "File writes are unavailable.")?;
        crate::native_git_effects::pull(&app, &host, &root, rebase)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn git_discard(
    window: Window,
    webview: tauri::Webview,
    app: tauri::AppHandle,
    root: String,
    change: crate::git::Change,
) -> Result<FileChange, String> {
    main_window(&window)?;
    crate::shared_host::trusted_full_window(&window, &webview)?;
    let deadline = std::time::Instant::now()
        .checked_add(std::time::Duration::from_secs(120))
        .ok_or("Git operation deadline is unavailable.")?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = window.state::<super::editor::EditorFiles>();
        let _lock = state
            .writes
            .lock()
            .map_err(|_| "File writes are unavailable.")?;
        crate::desktop_file_effects::git_discard(
            &app,
            &window,
            &webview,
            &root,
            change.into(),
            deadline,
        )
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn ignore_project_item(
    window: Window,
    webview: tauri::Webview,
    app: tauri::AppHandle,
    root: String,
    relative: String,
    local: bool,
) -> Result<FileChange, String> {
    main_window(&window)?;
    crate::shared_host::trusted_full_window(&window, &webview)?;
    let deadline = std::time::Instant::now()
        .checked_add(std::time::Duration::from_secs(120))
        .ok_or("Git operation deadline is unavailable.")?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = window.state::<super::editor::EditorFiles>();
        let _lock = state
            .writes
            .lock()
            .map_err(|_| "File writes are unavailable.")?;
        crate::desktop_file_effects::git_ignore(
            &app, &window, &webview, &root, &relative, local, deadline,
        )
    })
    .await
    .map_err(|error| error.to_string())?
}
