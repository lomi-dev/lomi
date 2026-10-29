use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use std::{
    collections::HashSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tauri::{Emitter, Manager, Window};
use tauri_plugin_dialog::DialogExt;

const FILE_LIMIT: u64 = 16 * 1024 * 1024;

#[derive(Default)]
struct FileWatch {
    watcher: Option<RecommendedWatcher>,
    directories: HashSet<PathBuf>,
}

#[derive(Default)]
pub struct EditorFiles {
    pub(super) writes: Mutex<()>,
    watch: Mutex<FileWatch>,
    paths: Arc<Mutex<HashSet<PathBuf>>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorError {
    kind: &'static str,
    message: String,
}

impl EditorError {
    pub(crate) fn new(kind: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
    pub(crate) fn io(error: impl std::fmt::Display) -> Self {
        Self::new("io", error.to_string())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum Encoding {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorFile {
    path: String,
    relative: String,
    content: Option<String>,
    revision: String,
    encoding: Encoding,
    read_only: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct FileLocation {
    root: String,
    relative: String,
}

#[derive(Debug, Serialize)]
pub struct SavedEditorFile {
    location: FileLocation,
    file: EditorFile,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveFile {
    root: String,
    relative: String,
    content: String,
    revision: String,
}

fn revision(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(unix)]
fn same_file_version(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    a.is_file()
        && b.is_file()
        && a.dev() == b.dev()
        && a.ino() == b.ino()
        && a.len() == b.len()
        && a.nlink() == b.nlink()
        && (a.mtime(), a.mtime_nsec(), a.ctime(), a.ctime_nsec())
            == (b.mtime(), b.mtime_nsec(), b.ctime(), b.ctime_nsec())
}
fn read_bytes(path: &Path) -> Result<(Vec<u8>, fs::Metadata), EditorError> {
    let mut file = super::resolved::open_resolved_file(path).map_err(EditorError::io)?;
    let metadata = file.metadata().map_err(EditorError::io)?;
    if !metadata.is_file() {
        return Err(EditorError::new(
            "unsupported",
            "Only regular text files can be edited.",
        ));
    }
    if metadata.len() > FILE_LIMIT {
        return Err(EditorError::new(
            "tooLarge",
            "This file exceeds the 16 MiB editor limit.",
        ));
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(FILE_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(EditorError::io)?;
    if bytes.len() as u64 > FILE_LIMIT {
        return Err(EditorError::new(
            "tooLarge",
            "This file exceeds the 16 MiB editor limit.",
        ));
    }
    #[cfg(unix)]
    {
        let after = file.metadata().map_err(EditorError::io)?;
        let named = super::resolved::open_resolved_file(path)
            .map_err(EditorError::io)?
            .metadata()
            .map_err(EditorError::io)?;
        if !same_file_version(&metadata, &after) || !same_file_version(&metadata, &named) {
            return Err(EditorError::new(
                "conflict",
                "The file changed while reading. Try again.",
            ));
        }
    }
    Ok((bytes, metadata))
}

pub(super) fn decode(bytes: &[u8]) -> Result<(String, Encoding), EditorError> {
    let invalid = || {
        EditorError::new(
            "unsupported",
            "This file is not supported text. Use UTF-8 or UTF-16 with a byte order mark.",
        )
    };
    let (text, encoding) = if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
        if !bytes.len().is_multiple_of(2) {
            return Err(invalid());
        }
        let little = bytes[0] == 0xff;
        let units: Vec<u16> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| {
                if little {
                    u16::from_le_bytes([pair[0], pair[1]])
                } else {
                    u16::from_be_bytes([pair[0], pair[1]])
                }
            })
            .collect();
        (
            String::from_utf16(&units).map_err(|_| invalid())?,
            if little {
                Encoding::Utf16Le
            } else {
                Encoding::Utf16Be
            },
        )
    } else {
        let (data, encoding) = if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
            (&bytes[3..], Encoding::Utf8Bom)
        } else {
            (bytes, Encoding::Utf8)
        };
        (
            std::str::from_utf8(data).map_err(|_| invalid())?.to_owned(),
            encoding,
        )
    };
    if text
        .chars()
        .any(|ch| ch == '\0' || (ch < ' ' && !matches!(ch, '\n' | '\r' | '\t' | '\u{000c}')))
    {
        return Err(EditorError::new(
            "unsupported",
            "Binary files cannot be edited as text.",
        ));
    }
    Ok((text, encoding))
}

pub(super) fn encode(text: &str, encoding: Encoding) -> Result<Vec<u8>, EditorError> {
    let mut bytes = match encoding {
        Encoding::Utf8 => Vec::new(),
        Encoding::Utf8Bom => vec![0xef, 0xbb, 0xbf],
        Encoding::Utf16Le => vec![0xff, 0xfe],
        Encoding::Utf16Be => vec![0xfe, 0xff],
    };
    if matches!(encoding, Encoding::Utf8 | Encoding::Utf8Bom) {
        bytes.extend_from_slice(text.as_bytes());
    } else {
        for unit in text.encode_utf16() {
            bytes.extend_from_slice(&if encoding == Encoding::Utf16Le {
                unit.to_le_bytes()
            } else {
                unit.to_be_bytes()
            });
        }
    }
    if bytes.len() as u64 > FILE_LIMIT {
        return Err(EditorError::new(
            "tooLarge",
            "The edited file exceeds the 16 MiB save limit. Your changes are still in the editor.",
        ));
    }
    Ok(bytes)
}

fn resolve(root: &str, relative: &str) -> Result<(PathBuf, String), EditorError> {
    let path = super::inside(root, relative).map_err(EditorError::io)?;
    if !path.is_file() {
        return Err(EditorError::new(
            "unsupported",
            "Only regular files can be opened in the editor.",
        ));
    }
    let root = super::directory(root).map_err(EditorError::io)?;
    let relative = path
        .strip_prefix(root)
        .map_err(EditorError::io)?
        .to_str()
        .ok_or_else(|| EditorError::new("unsupported", "This filename is not valid Unicode."))?
        .to_owned();
    Ok((path, relative))
}

fn read(root: &str, relative: &str, known: Option<&str>) -> Result<EditorFile, EditorError> {
    let (path, relative) = resolve(root, relative)?;
    let (bytes, metadata) = read_bytes(&path)?;
    let hash = revision(&bytes);
    let (text, encoding) = decode(&bytes)?;
    Ok(EditorFile {
        path: path
            .to_str()
            .ok_or_else(|| EditorError::new("unsupported", "This filename is not valid Unicode."))?
            .to_owned(),
        relative,
        content: (known != Some(hash.as_str())).then_some(text),
        revision: hash,
        encoding,
        read_only: metadata.permissions().readonly(),
    })
}

fn write_host(app: &tauri::AppHandle, request: &SaveFile) -> Result<String, EditorError> {
    use lomi_remote_protocol::{
        ipc::{LocalNativeEditorEffect, LocalNativeRelativePath, LocalRootPath},
        HostKeyCommitmentHex,
    };

    let (path, relative) = resolve(&request.root, &request.relative)?;
    let effect = LocalNativeEditorEffect::Save {
        root: LocalRootPath::try_from(request.root.clone())
            .map_err(|_| EditorError::new("unsupported", "The project folder path is invalid."))?,
        relative: LocalNativeRelativePath::try_from(relative.clone())
            .map_err(|_| EditorError::new("unsupported", "The editor file path is invalid."))?,
        expected_revision: HostKeyCommitmentHex::try_from(request.revision.clone())
            .map_err(|_| EditorError::new("conflict", "The file revision is invalid."))?,
    };
    if let Some(result) = crate::native_file_effects::recover_pending(
        app,
        &path,
        &request.content,
        &effect,
        crate::native_file_effects::ExpectedResult::File,
    )? {
        return match result {
            lomi_native_files::NativeSaveResult::File(file)
                if file.path == path.to_str().unwrap_or_default()
                    && file.relative == relative =>
            {
                let current = read_bytes(&path).map_err(|_| {
                    EditorError::new(
                        "conflict",
                        "The file changed after the previous save. Your changes remain in the editor.",
                    )
                })?;
                if revision(&current.0) != file.revision {
                    return Err(EditorError::new(
                        "conflict",
                        "The file changed after the previous save. Your changes remain in the editor.",
                    ));
                }
                Ok(file.revision)
            }
            _ => Err(EditorError::new(
                "uncertain",
                "The host completed the save but returned invalid file metadata. Your changes remain in the editor.",
            )),
        };
    }
    let _admission = crate::project_write_guard::admit(&[&path])
        .map_err(|message| EditorError::new("codingRunActive", message))?;
    let (original, metadata) = read_bytes(&path)?;
    if revision(&original) != request.revision {
        return Err(EditorError::new(
            "conflict",
            "The file changed on disk. Reload it or explicitly overwrite the disk version.",
        ));
    }
    if metadata.permissions().readonly() {
        return Err(EditorError::new(
            "readOnly",
            "This file is read-only. Your changes are still in the editor.",
        ));
    }
    let (_, encoding) = decode(&original)?;
    // Preserve the existing on-disk encoding and limit before staging raw UTF-8.
    let _ = encode(&request.content, encoding)?;
    let result = crate::native_file_effects::save(
        app,
        &path,
        &request.content,
        effect,
        crate::native_file_effects::ExpectedResult::File,
    )?;
    match result {
        lomi_native_files::NativeSaveResult::File(file)
            if file.path == path.to_str().unwrap_or_default() && file.relative == relative =>
        {
            Ok(file.revision)
        }
        _ => Err(EditorError::new(
            "uncertain",
            "The host completed the save but returned invalid file metadata. Your changes remain in the editor.",
        )),
    }
}

fn canonical_new_path(path: &Path) -> Result<PathBuf, EditorError> {
    if path.try_exists().map_err(EditorError::io)? {
        fs::canonicalize(path).map_err(EditorError::io)
    } else {
        let parent = path
            .parent()
            .ok_or_else(|| EditorError::new("io", "The file has no parent directory."))?;
        let name = path
            .file_name()
            .ok_or_else(|| EditorError::new("io", "Choose a filename."))?;
        fs::canonicalize(parent)
            .map_err(EditorError::io)
            .map(|parent| parent.join(name))
    }
}

fn write_new_host(
    app: &tauri::AppHandle,
    selected_path: &Path,
    content: &str,
    open_files: &[FileLocation],
) -> Result<SavedEditorFile, EditorError> {
    use lomi_remote_protocol::{
        ipc::{
            LocalNativeEditorEffect, LocalNativeFileLocation, LocalNativeRelativePath,
            LocalRootPath,
        },
        rpc::BoundedList,
    };

    let path = canonical_new_path(selected_path)?;
    let parent = path
        .parent()
        .ok_or_else(|| EditorError::new("io", "The file has no parent directory."))?;
    let unicode = || EditorError::new("unsupported", "This filename is not valid Unicode.");
    let root = parent.to_str().ok_or_else(unicode)?.to_owned();
    let relative = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(unicode)?
        .to_owned();
    if open_files.len() > 256 {
        return Err(EditorError::new(
            "tooLarge",
            "Too many editor files are open to safely save this file.",
        ));
    }
    let native_open_files = open_files
        .iter()
        .map(|file| {
            Ok(LocalNativeFileLocation {
                root: LocalRootPath::try_from(file.root.clone()).map_err(|_| {
                    EditorError::new("unsupported", "An open editor folder path is invalid.")
                })?,
                relative: LocalNativeRelativePath::try_from(file.relative.clone()).map_err(
                    |_| EditorError::new("unsupported", "An open editor file path is invalid."),
                )?,
            })
        })
        .collect::<Result<Vec<_>, EditorError>>()?;
    let absolute_path = LocalRootPath::try_from(path.to_str().ok_or_else(unicode)?.to_owned())
        .map_err(|_| EditorError::new("unsupported", "The selected file path is invalid."))?;
    let legacy_effect = LocalNativeEditorEffect::SaveNew {
        absolute_path,
        open_files: BoundedList(native_open_files),
    };
    if let Some(result) = crate::native_file_effects::recover_pending_new_checked(
        app,
        &path,
        content,
        &legacy_effect,
        crate::native_file_effects::ExpectedResult::New,
    )? {
        return match result {
            lomi_native_files::NativeSaveResult::New(saved)
                if saved.file.path == path.to_str().unwrap_or_default()
                    && saved.file.relative == relative
                    && saved.location.root == root
                    && saved.location.relative == relative =>
            {
                let current_revision = read(&root, &relative, None)
                    .map_err(|_| {
                        EditorError::new(
                            "conflict",
                            "The file changed after the previous save. Your changes remain in the editor.",
                        )
                    })?
                    .revision;
                if current_revision != saved.file.revision {
                    return Err(EditorError::new(
                        "conflict",
                        "The file changed after the previous save. Your changes remain in the editor.",
                    ));
                }
                Ok(saved_editor_file_from_native(saved))
            }
            _ => Err(EditorError::new(
                "uncertain",
                "The host completed the save but returned invalid file metadata. Your changes remain in the editor.",
            )),
        };
    }
    if open_files
        .iter()
        .any(|file| super::inside(&file.root, &file.relative).is_ok_and(|open| open == path))
    {
        return Err(EditorError::new(
            "openFile",
            "This file is already open. Close its editor tabs before replacing it.",
        ));
    }
    let _admission = crate::project_write_guard::admit(&[&path])
        .map_err(|message| EditorError::new("codingRunActive", message))?;
    let expected_revision = if path.try_exists().map_err(EditorError::io)? {
        if !path.is_file() {
            return Err(EditorError::new(
                "unsupported",
                "Only regular files can be opened in the editor.",
            ));
        }
        let existing = read(&root, &relative, None)?;
        if existing.read_only {
            return Err(EditorError::new(
                "readOnly",
                "This file is read-only. Your changes are still in the editor.",
            ));
        }
        let _ = encode(content, existing.encoding)?;
        Some(existing.revision)
    } else {
        let _ = encode(content, Encoding::Utf8)?;
        None
    };
    let expected_revision = expected_revision
        .map(lomi_remote_protocol::HostKeyCommitmentHex::try_from)
        .transpose()
        .map_err(|_| EditorError::new("unsupported", "The file revision is invalid."))?;
    let checked_effect = match legacy_effect {
        LocalNativeEditorEffect::SaveNew {
            absolute_path,
            open_files,
        } => LocalNativeEditorEffect::SaveNewChecked {
            absolute_path,
            open_files,
            expected_revision,
        },
        _ => unreachable!("legacy SaveAs effect has the SaveNew variant"),
    };
    let result = crate::native_file_effects::save(
        app,
        &path,
        content,
        checked_effect,
        crate::native_file_effects::ExpectedResult::New,
    )?;
    match result {
        lomi_native_files::NativeSaveResult::New(saved)
            if saved.file.path == path.to_str().unwrap_or_default()
                && saved.file.relative == relative
                && saved.location.root == root
                && saved.location.relative == relative =>
        {
            Ok(saved_editor_file_from_native(saved))
        }
        _ => Err(EditorError::new(
            "uncertain",
            "The host completed the save but returned invalid file metadata. Your changes remain in the editor.",
        )),
    }
}

fn saved_editor_file_from_native(
    saved: lomi_native_files::NativeSavedEditorFile,
) -> SavedEditorFile {
    SavedEditorFile {
        location: FileLocation {
            root: saved.location.root,
            relative: saved.location.relative,
        },
        file: editor_file_from_native(saved.file),
    }
}

fn editor_file_from_native(file: lomi_native_files::NativeEditorFile) -> EditorFile {
    EditorFile {
        path: file.path,
        relative: file.relative,
        content: file.content,
        revision: file.revision,
        encoding: match file.encoding {
            lomi_native_files::NativeFileEncoding::Utf8 => Encoding::Utf8,
            lomi_native_files::NativeFileEncoding::Utf8Bom => Encoding::Utf8Bom,
            lomi_native_files::NativeFileEncoding::Utf16Le => Encoding::Utf16Le,
            lomi_native_files::NativeFileEncoding::Utf16Be => Encoding::Utf16Be,
        },
        read_only: file.read_only,
    }
}

#[tauri::command]
pub async fn save_new_editor_file(
    window: Window,
    app: tauri::AppHandle,
    content: String,
    suggested_name: String,
    open_files: Vec<FileLocation>,
) -> Result<Option<SavedEditorFile>, EditorError> {
    super::main_window(&window).map_err(EditorError::io)?;
    tauri::async_runtime::spawn_blocking(move || {
        let Some(path) = app
            .dialog()
            .file()
            .set_parent(&window)
            .set_title("Save file")
            .set_file_name(suggested_name)
            .blocking_save_file()
        else {
            return Ok(None);
        };
        let path = path.into_path().map_err(EditorError::io)?;
        let state = app.state::<EditorFiles>();
        let _guard = state.writes.lock().map_err(EditorError::io)?;
        write_new_host(&app, &path, &content, &open_files).map(Some)
    })
    .await
    .map_err(EditorError::io)?
}

#[tauri::command]
pub async fn resolve_editor_file(
    window: Window,
    root: String,
    relative: String,
) -> Result<String, EditorError> {
    super::main_window(&window).map_err(EditorError::io)?;
    tauri::async_runtime::spawn_blocking(move || {
        resolve(&root, &relative).map(|(_, relative)| relative)
    })
    .await
    .map_err(EditorError::io)?
}

#[tauri::command]
pub async fn read_editor_file(
    window: Window,
    root: String,
    relative: String,
    known_revision: Option<String>,
) -> Result<EditorFile, EditorError> {
    super::main_window(&window).map_err(EditorError::io)?;
    tauri::async_runtime::spawn_blocking(move || read(&root, &relative, known_revision.as_deref()))
        .await
        .map_err(EditorError::io)?
}

#[tauri::command]
pub async fn save_editor_file(
    window: Window,
    app: tauri::AppHandle,
    request: SaveFile,
) -> Result<String, EditorError> {
    super::main_window(&window).map_err(EditorError::io)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<EditorFiles>();
        let _guard = state.writes.lock().map_err(EditorError::io)?;
        write_host(&app, &request)
    })
    .await
    .map_err(EditorError::io)?
}

#[tauri::command]
pub async fn watch_editor_files(
    window: Window,
    app: tauri::AppHandle,
    files: Vec<FileLocation>,
) -> Result<(), String> {
    super::main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<EditorFiles>();
        let paths: HashSet<PathBuf> = files
            .iter()
            .filter_map(|file| super::inside(&file.root, &file.relative).ok())
            .collect();
        let directories: HashSet<PathBuf> = paths
            .iter()
            .filter_map(|path| path.parent().map(Path::to_path_buf))
            .collect();
        let mut watch = state.watch.lock().map_err(|error| error.to_string())?;
        *state.paths.lock().map_err(|error| error.to_string())? = paths;
        if directories.is_empty() {
            *watch = FileWatch::default();
            return Ok(());
        }
        if watch.watcher.is_none() {
            let app = app.clone();
            let paths = state.paths.clone();
            watch.watcher = Some(
                notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                    if let Ok(ref event) = event {
                        if matches!(event.kind, EventKind::Access(_)) && !event.need_rescan() {
                            return;
                        }
                    }
                    if let Ok(paths) = paths.lock() {
                        let changed: Vec<String> = paths
                            .iter()
                            .filter(|file| {
                                event.as_ref().map_or(true, |event| {
                                    event.need_rescan()
                                        || event.paths.is_empty()
                                        || event
                                            .paths
                                            .iter()
                                            .any(|path| *file == path || file.starts_with(path))
                                })
                            })
                            .map(|path| path.to_string_lossy().into_owned())
                            .collect();
                        if !changed.is_empty() {
                            let _ = app.emit_to("main", "editor-files-changed", changed);
                        }
                    }
                })
                .map_err(|error| error.to_string())?,
            );
        }
        let removed: Vec<_> = watch
            .directories
            .difference(&directories)
            .cloned()
            .collect();
        let added: Vec<_> = directories
            .difference(&watch.directories)
            .cloned()
            .collect();
        for directory in removed {
            let _ = watch.watcher.as_mut().unwrap().unwatch(&directory);
            watch.directories.remove(&directory);
        }
        for directory in added {
            watch
                .watcher
                .as_mut()
                .unwrap()
                .watch(&directory, RecursiveMode::NonRecursive)
                .map_err(|error| error.to_string())?;
            watch.directories.insert(directory);
        }
        Ok(())
    })
    .await
    .map_err(|error| error.to_string())?
}
