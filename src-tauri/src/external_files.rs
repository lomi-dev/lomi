use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::Mutex,
};
use tauri::{AppHandle, Emitter, Manager, State, Window};

#[derive(Clone, serde::Serialize)]
pub(crate) struct Request {
    id: u64,
    path: String,
}

#[derive(Default)]
pub(crate) struct Requests(Mutex<Queue>);

#[derive(Default)]
struct Queue {
    ready: bool,
    sequence: u64,
    pending: Vec<Request>,
}

impl Queue {
    fn push(&mut self, paths: Vec<PathBuf>) {
        for path in paths {
            let Some(path) = path.to_str() else { continue };
            let Some(id) = self.sequence.checked_add(1) else {
                break;
            };
            self.sequence = id;
            self.pending.push(Request {
                id,
                path: path.into(),
            });
        }
    }

    fn ack(&mut self, ids: &[u64]) {
        self.pending.retain(|request| !ids.contains(&request.id));
    }
}

// Resolve relative launcher arguments in the launching process, before forwarding
// them to an instance whose working directory may be different.
pub(crate) fn arguments(args: impl IntoIterator<Item = OsString>, cwd: &Path) -> Vec<PathBuf> {
    let mut positional = false;
    args.into_iter()
        .filter_map(|arg| {
            if arg == "--" && !positional {
                positional = true;
                return None;
            }
            if !positional && arg.to_string_lossy().starts_with('-') {
                return None;
            }
            let path = if arg.to_string_lossy().starts_with("file:") {
                tauri::Url::parse(arg.to_str()?).ok()?.to_file_path().ok()?
            } else {
                PathBuf::from(arg)
            };
            if path.as_os_str().is_empty() {
                return None;
            }
            Some(if path.is_absolute() {
                path
            } else {
                cwd.join(path)
            })
        })
        .collect()
}

pub(crate) fn enqueue(app: &AppHandle, paths: Vec<PathBuf>) {
    if paths.is_empty() {
        return;
    }
    let ready = {
        let requests = app.state::<Requests>();
        let mut queue = requests.0.lock().unwrap();
        queue.push(paths);
        queue.ready
    };
    #[cfg(target_os = "macos")]
    if ready {
        if let Some(window) = app.get_window("main") {
            let _ = window.unminimize();
            let _ = window.show();
            let _ = window.set_focus();
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = ready;
    // Requests remain queued until acknowledged; the first frontend subscription
    // also queries the queue to recover events sent before session restoration.
    let _ = app.emit_to(
        tauri::EventTarget::webview("main"),
        "external-files-pending",
        (),
    );
}

pub(crate) fn ready(app: &AppHandle) {
    app.state::<Requests>().0.lock().unwrap().ready = true;
}

#[tauri::command]
pub(crate) fn external_files_pending(
    window: Window,
    requests: State<'_, Requests>,
) -> Result<Vec<Request>, String> {
    crate::files::main_window(&window)?;
    Ok(requests
        .0
        .lock()
        .map_err(|error| error.to_string())?
        .pending
        .clone())
}

#[tauri::command]
pub(crate) fn external_files_ack(
    window: Window,
    requests: State<'_, Requests>,
    ids: Vec<u64>,
) -> Result<(), String> {
    crate::files::main_window(&window)?;
    requests
        .0
        .lock()
        .map_err(|error| error.to_string())?
        .ack(&ids);
    Ok(())
}

#[derive(Debug, PartialEq, serde::Serialize)]
pub(crate) struct Destination {
    root: String,
    relative: String,
}

fn resolve(path: &Path, roots: &[String]) -> Result<Destination, String> {
    let path = std::fs::canonicalize(path)
        .map_err(|error| format!("Cannot open {}: {error}", path.display()))?;
    if !path.is_file() {
        return Err("Only regular files can be opened in the editor.".into());
    }
    let matched = roots
        .iter()
        .filter_map(|root| {
            let canonical = std::fs::canonicalize(root).ok()?;
            let relative = path.strip_prefix(&canonical).ok()?;
            if relative.as_os_str().is_empty() {
                return None;
            }
            Some((
                canonical.components().count(),
                root.clone(),
                relative.to_owned(),
            ))
        })
        .max_by_key(|(depth, _, _)| *depth);
    let (root, relative) = if let Some((_, root, relative)) = matched {
        (root, relative)
    } else {
        let parent = path.parent().ok_or("This file has no parent directory.")?;
        (
            parent
                .to_str()
                .ok_or("The file path is not valid Unicode.")?
                .into(),
            path.strip_prefix(parent)
                .map_err(|error| error.to_string())?
                .to_owned(),
        )
    };
    let relative = relative
        .to_str()
        .ok_or("The file path is not valid Unicode.")?
        .to_owned();
    Ok(Destination { root, relative })
}

#[tauri::command]
pub(crate) async fn resolve_external_file(
    window: Window,
    path: String,
    roots: Vec<String>,
) -> Result<Destination, String> {
    crate::files::main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || resolve(Path::new(&path), &roots))
        .await
        .map_err(|error| error.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_preserve_spaces_unicode_relative_paths_and_file_urls() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("space żółć #%.txt");
        let url = tauri::Url::from_file_path(&path).unwrap();
        let args = [
            OsString::from("--ignored"),
            OsString::from("relative file"),
            path.clone().into_os_string(),
            OsString::from(url.as_str()),
            OsString::from("--"),
            OsString::from("-notes"),
        ];
        assert_eq!(
            arguments(args, dir.path()),
            vec![
                dir.path().join("relative file"),
                path.clone(),
                path,
                dir.path().join("-notes")
            ]
        );
    }

    #[test]
    fn queue_keeps_early_requests_until_their_own_acknowledgment() {
        let mut queue = Queue::default();
        queue.push(vec![PathBuf::from("one"), PathBuf::from("two")]);
        let first = queue.pending[0].id;
        queue.push(vec![PathBuf::from("three")]);
        assert_eq!(queue.pending.len(), 3);
        queue.ack(&[first]);
        assert_eq!(
            queue
                .pending
                .iter()
                .map(|r| r.path.as_str())
                .collect::<Vec<_>>(),
            ["two", "three"]
        );
        assert!(queue.pending[0].id < queue.pending[1].id);
    }

    #[test]
    fn resolves_deepest_project_and_falls_back_to_file_parent() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("nested");
        std::fs::create_dir(&nested).unwrap();
        let file = nested.join(".env");
        std::fs::write(&file, b"TEST=1").unwrap();
        let parent = std::fs::canonicalize(dir.path())
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        let root = std::fs::canonicalize(&nested)
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        assert_eq!(
            resolve(&file, &[parent.clone(), root.clone()]).unwrap(),
            Destination {
                root: root.clone(),
                relative: ".env".into()
            }
        );
        assert_eq!(
            resolve(&file, &[]).unwrap(),
            Destination {
                root,
                relative: ".env".into()
            }
        );
        assert!(resolve(&nested, &[parent]).is_err());
        assert!(resolve(&nested.join("missing"), &[]).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn canonicalizes_symlinks_and_preserves_unix_backslashes() {
        let dir = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let file = other.path().join("has\\backslash");
        std::fs::write(&file, b"text").unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        let resolved = resolve(&link, &[dir.path().to_str().unwrap().into()]).unwrap();
        assert_eq!(
            resolved.root,
            std::fs::canonicalize(other.path())
                .unwrap()
                .to_str()
                .unwrap()
        );
        assert_eq!(resolved.relative, "has\\backslash");
    }
}
