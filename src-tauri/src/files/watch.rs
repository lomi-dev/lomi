use notify::{event::ModifyKind, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{mpsc, Arc, Mutex},
    time::Duration,
};
use tauri::{ipc::Channel, Manager, Resource, ResourceId, Webview, Window};

struct DirectoryWatch {
    _watcher: RecommendedWatcher,
}

impl Resource for DirectoryWatch {}

fn changed_directories(
    event: notify::Result<notify::Event>,
    directories: &HashMap<PathBuf, Vec<String>>,
) -> HashSet<String> {
    let Ok(event) = event else {
        return directories.values().flatten().cloned().collect();
    };
    if event.need_rescan() || event.paths.is_empty() {
        return directories.values().flatten().cloned().collect();
    }
    if matches!(
        event.kind,
        EventKind::Access(_) | EventKind::Modify(ModifyKind::Data(_) | ModifyKind::Metadata(_))
    ) {
        return HashSet::new();
    }
    // ReadDirectoryChangesW reports content/attribute writes as Modify(Any).
    // Create, remove and rename have distinct events and still refresh listings.
    #[cfg(windows)]
    if matches!(event.kind, EventKind::Modify(ModifyKind::Any)) {
        return HashSet::new();
    }
    event
        .paths
        .iter()
        .filter(|path| path.file_name().is_none_or(|name| name != ".git"))
        .flat_map(|path| [Some(path.as_path()), path.parent()])
        .flatten()
        .filter_map(|path| directories.get(path))
        .flatten()
        .cloned()
        .collect()
}

fn watch_directories(
    root: &str,
    relatives: Vec<String>,
    on_change: impl Fn(Vec<String>) + Send + 'static,
) -> Result<DirectoryWatch, String> {
    let mut directories = HashMap::<PathBuf, Vec<String>>::new();
    directories.insert(super::inside(root, "")?, vec![String::new()]);
    for relative in relatives.into_iter().filter(|path| !path.is_empty()) {
        // Expanded folders may disappear before their parent's next listing.
        if let Ok(path) = super::inside(root, &relative) {
            if path.is_dir() {
                directories.entry(path).or_default().push(relative);
            }
        }
    }
    let pending = Arc::new(Mutex::new(HashSet::<String>::new()));
    let (wake, events) = mpsc::sync_channel(1);
    let changed = pending.clone();
    let watched = directories.clone();
    let mut watcher = notify::recommended_watcher(move |event| {
        let directories = changed_directories(event, &watched);
        if directories.is_empty() {
            return;
        }
        if let Ok(mut pending) = changed.lock() {
            pending.extend(directories);
            let _ = wake.try_send(());
        }
    })
    .map_err(|error| error.to_string())?;
    for path in directories.keys() {
        if let Err(error) = watcher.watch(path, RecursiveMode::NonRecursive) {
            if path.exists() || directories[path].iter().any(String::is_empty) {
                return Err(error.to_string());
            }
        }
    }
    std::thread::Builder::new()
        .name("explorer-watch".into())
        .spawn(move || {
            while events.recv().is_ok() {
                // Fixed batches keep continuous writes responsive and bound IPC traffic.
                std::thread::sleep(Duration::from_millis(100));
                let Ok(mut pending) = pending.lock() else {
                    break;
                };
                while events.try_recv().is_ok() {}
                let changed: Vec<_> = pending.drain().collect();
                drop(pending);
                if !changed.is_empty() {
                    on_change(changed);
                }
            }
        })
        .map_err(|error| error.to_string())?;
    Ok(DirectoryWatch { _watcher: watcher })
}

#[tauri::command]
pub async fn watch_explorer_directories(
    window: Window,
    webview: Webview,
    root: String,
    relatives: Vec<String>,
    on_change: Channel<Vec<String>>,
) -> Result<ResourceId, String> {
    super::main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        let watcher = watch_directories(&root, relatives, move |changed| {
            let _ = on_change.send(changed);
        })?;
        Ok(webview.resources_table().add(watcher))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::Path};

    #[test]
    fn batches_native_changes_only_for_visible_directories_and_stops_on_drop() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("expanded")).unwrap();
        fs::create_dir(root.path().join("collapsed")).unwrap();
        let (send, receive) = mpsc::channel();
        let watcher = watch_directories(
            root.path().to_str().unwrap(),
            vec!["expanded".into(), "missing".into(), "../".into()],
            move |changed| send.send(changed).unwrap(),
        )
        .unwrap();
        for i in 0..50 {
            fs::write(root.path().join(format!("expanded/{i}.txt")), "test").unwrap();
        }
        let changed = receive.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(changed, ["expanded"]);
        while receive.recv_timeout(Duration::from_millis(200)).is_ok() {}
        fs::write(root.path().join("collapsed/hidden.txt"), "test").unwrap();
        fs::write(root.path().join("expanded/0.txt"), "content only").unwrap();
        // FSEvents can coalesce creation and content writes into another Create event.
        // Such refreshes may name the visible folder, but never the collapsed folder.
        // The synthetic event test below verifies that Data events are ignored.
        while let Ok(changed) = receive.recv_timeout(Duration::from_millis(250)) {
            assert_eq!(changed, ["expanded"]);
        }
        fs::create_dir(root.path().join("new folder")).unwrap();
        assert_eq!(receive.recv_timeout(Duration::from_secs(5)).unwrap(), [""]);
        fs::rename(root.path().join("new folder"), root.path().join("renamed")).unwrap();
        assert_eq!(receive.recv_timeout(Duration::from_secs(5)).unwrap(), [""]);
        fs::remove_dir(root.path().join("renamed")).unwrap();
        assert_eq!(receive.recv_timeout(Duration::from_secs(5)).unwrap(), [""]);
        drop(watcher);
        assert!(matches!(
            receive.recv_timeout(Duration::from_secs(5)),
            Err(mpsc::RecvTimeoutError::Disconnected)
        ));
    }

    #[test]
    fn handles_renames_rescans_and_ignores_content_and_git_events() {
        let directories = HashMap::from([
            (Path::new("/project").into(), vec![String::new()]),
            (Path::new("/project/src").into(), vec!["src".into()]),
        ]);
        let event = notify::Event::new(EventKind::Modify(ModifyKind::Name(
            notify::event::RenameMode::Both,
        )))
        .add_path("/project/file".into())
        .add_path("/project/src/file".into());
        assert_eq!(
            changed_directories(Ok(event), &directories),
            HashSet::from([String::new(), "src".into()])
        );
        for kind in [
            EventKind::Access(notify::event::AccessKind::Any),
            EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Any)),
            #[cfg(windows)]
            EventKind::Modify(ModifyKind::Any),
        ] {
            assert!(changed_directories(
                Ok(notify::Event::new(kind).add_path("/project/file".into())),
                &directories
            )
            .is_empty());
        }
        assert!(changed_directories(
            Ok(notify::Event::new(EventKind::Any).add_path("/project/.git".into())),
            &directories
        )
        .is_empty());
        assert_eq!(
            changed_directories(
                Ok(notify::Event::new(EventKind::Other).set_flag(notify::event::Flag::Rescan)),
                &directories
            )
            .len(),
            2
        );
    }
}
