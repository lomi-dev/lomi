use std::{
    fs::{self, File, OpenOptions},
    io,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::Path,
    sync::Mutex,
    time::{Duration, Instant},
};
use tauri::{plugin::Plugin, AppHandle, Manager, RunEvent, Wry};

pub(crate) fn wait_for_previous_owner(app: &AppHandle) -> io::Result<()> {
    let data = app.path().app_data_dir().map_err(io::Error::other)?;
    wait_for_owners(&data, Duration::from_secs(5), || {
        #[cfg(feature = "native-smoke")]
        crate::native_smoke::linux_instance_owner_contended(app)?;
        Ok(())
    })
}

fn wait_for_owners(
    data: &Path,
    timeout: Duration,
    on_contention: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    let mut on_contention = Some(on_contention);
    let deadline = Instant::now() + timeout;
    loop {
        // Keep both probes alive together. A failed attempt drops any partial
        // acquisition before sleeping, so it cannot obstruct the retiring owner.
        let result: io::Result<_> = (|| {
            let runtime = probe_owner(&data.join("agent-runtime"))?;
            let chat = probe_owner(&data.join("chat-ai"))?;
            Ok((runtime, chat))
        })();
        match result {
            Ok(_owners) => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                if let Some(notify) = on_contention.take() {
                    notify()?;
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "The previous Lomi instance still owns application storage.",
                    ));
                }
                std::thread::sleep(remaining.min(Duration::from_millis(15)));
            }
            Err(error) => return Err(error),
        }
    }
}

fn probe_owner(root: &Path) -> io::Result<Option<File>> {
    let metadata = match fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(io::Error::other("Invalid Lomi storage directory."));
    }
    fs::set_permissions(root, fs::Permissions::from_mode(0o700))?;
    let path = fs::canonicalize(root)?.join("owner.lock");
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(io::Error::other("Invalid Lomi storage owner lock."));
    }
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    if !lock.metadata()?.is_file() {
        return Err(io::Error::other("Invalid Lomi storage owner lock."));
    }
    lock.set_permissions(fs::Permissions::from_mode(0o600))?;
    lock.try_lock().map_err(|error| match error {
        std::fs::TryLockError::WouldBlock => io::Error::from(io::ErrorKind::WouldBlock),
        std::fs::TryLockError::Error(error) => error,
    })?;
    Ok(Some(lock))
}

#[derive(Default)]
pub(crate) struct Reopen(Mutex<Activation>);

#[derive(Default)]
struct Activation {
    ready: bool,
    pending: bool,
    scheduled: bool,
    stopped: bool,
}

impl Activation {
    fn schedule(&mut self) -> bool {
        if self.stopped || !self.ready || !self.pending || self.scheduled {
            return false;
        }
        self.scheduled = true;
        true
    }

    fn take(&mut self) -> bool {
        self.scheduled = false;
        let activate = !self.stopped && self.ready && self.pending;
        self.pending = false;
        activate
    }

    fn stop(&mut self) {
        self.stopped = true;
        self.pending = false;
    }
}

pub(crate) fn ready(app: &AppHandle) {
    let schedule = {
        let state = app.state::<Reopen>();
        let mut activation = state.0.lock().unwrap();
        activation.ready = true;
        activation.schedule()
    };
    if schedule {
        present(app);
    }
}

fn request(app: &AppHandle) {
    let schedule = {
        let state = app.state::<Reopen>();
        let mut activation = state.0.lock().unwrap();
        if activation.stopped {
            return;
        }
        activation.pending = true;
        activation.schedule()
    };
    if schedule {
        present(app);
    }
}

fn present(app: &AppHandle) {
    let handle = app.clone();
    if let Err(error) = app.run_on_main_thread(move || {
        if !handle.state::<Reopen>().0.lock().unwrap().take() {
            return;
        }
        if let Some(window) = handle.get_window("main") {
            use gtk::prelude::*;
            let _ = window.unminimize();
            let _ = window.show();
            let _ = window.set_focus();
            // Tao queues show(), but its focus path checks GTK visibility before
            // that queue runs. Present the owned GTK window on its main thread.
            if let Ok(native) = window.gtk_window() {
                native.present_with_time(0);
            }
        }
    }) {
        app.state::<Reopen>().0.lock().unwrap().scheduled = false;
        eprintln!("Cannot schedule Lomi reopen: {error}");
    }
}

pub(crate) fn plugin() -> impl Plugin<Wry> {
    DeferredExit(tauri_plugin_single_instance::init(|app, args, cwd| {
        crate::external_files::enqueue(
            app,
            crate::external_files::arguments(
                args.into_iter().skip(1).map(Into::into),
                Path::new(&cwd),
            ),
        );
        request(app);
    }))
}

struct DeferredExit(tauri::plugin::TauriPlugin<Wry>);

impl Plugin<Wry> for DeferredExit {
    fn name(&self) -> &'static str {
        self.0.name()
    }

    fn initialize(
        &mut self,
        app: &AppHandle,
        config: serde_json::Value,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.0.initialize(app, config)
    }

    fn on_event(&mut self, app: &AppHandle, event: &RunEvent) {
        match event {
            RunEvent::Exit => {
                app.state::<Reopen>().0.lock().unwrap().stop();
                // Plugin 2.4.2 uses only setup and on_event on Linux. Its Exit
                // hook releases D-Bus ownership before Lomi's run callback;
                // lib.rs explicitly destroys it after PTY and state cleanup.
            }
            RunEvent::WindowEvent {
                label,
                event: tauri::WindowEvent::Destroyed,
                ..
            } if label == "main" => {
                app.state::<Reopen>().0.lock().unwrap().stop();
                self.0.on_event(app, event);
            }
            _ => self.0.on_event(app, event),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner_file(data: &Path, name: &str) -> File {
        let root = data.join(name);
        fs::create_dir(&root).unwrap();
        let path = root.join("owner.lock");
        fs::write(&path, b"preserved owner data").unwrap();
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .unwrap()
    }

    #[test]
    fn startup_waits_until_both_existing_owners_release() {
        let temp = tempfile::tempdir().unwrap();
        let runtime = owner_file(temp.path(), "agent-runtime");
        let chat = owner_file(temp.path(), "chat-ai");
        runtime.try_lock().unwrap();
        chat.try_lock().unwrap();
        let data = temp.path().to_owned();
        let (finished, result) = std::sync::mpsc::channel();
        let waiter = std::thread::spawn(move || {
            finished
                .send(wait_for_owners(&data, Duration::from_secs(2), || Ok(())))
                .unwrap();
        });
        assert!(result.recv_timeout(Duration::from_millis(30)).is_err());
        runtime.unlock().unwrap();
        assert!(result.recv_timeout(Duration::from_millis(30)).is_err());
        chat.unlock().unwrap();
        result
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap();
        waiter.join().unwrap();
        runtime.try_lock().unwrap();
        chat.try_lock().unwrap();
        assert_eq!(
            fs::read(temp.path().join("chat-ai/owner.lock")).unwrap(),
            b"preserved owner data"
        );
    }

    #[test]
    fn timeout_releases_partial_probes_and_allows_retry() {
        let temp = tempfile::tempdir().unwrap();
        let runtime = owner_file(temp.path(), "agent-runtime");
        let chat = owner_file(temp.path(), "chat-ai");
        chat.try_lock().unwrap();
        assert_eq!(
            wait_for_owners(temp.path(), Duration::from_millis(35), || Ok(()))
                .unwrap_err()
                .kind(),
            io::ErrorKind::TimedOut
        );
        runtime.try_lock().unwrap();
        runtime.unlock().unwrap();
        chat.unlock().unwrap();
        wait_for_owners(temp.path(), Duration::ZERO, || Ok(())).unwrap();
        runtime.try_lock().unwrap();
        chat.try_lock().unwrap();
    }

    #[test]
    fn missing_owners_are_not_created() {
        let temp = tempfile::tempdir().unwrap();
        wait_for_owners(temp.path(), Duration::ZERO, || Ok(())).unwrap();
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
        fs::create_dir(temp.path().join("chat-ai")).unwrap();
        wait_for_owners(temp.path(), Duration::ZERO, || Ok(())).unwrap();
        assert!(!temp.path().join("chat-ai/owner.lock").exists());
    }

    #[test]
    fn contention_notification_runs_once_and_propagates_errors() {
        let temp = tempfile::tempdir().unwrap();
        let owner = owner_file(temp.path(), "chat-ai");
        owner.try_lock().unwrap();
        let mut notifications = 0;
        assert_eq!(
            wait_for_owners(temp.path(), Duration::from_millis(35), || {
                notifications += 1;
                Ok(())
            })
            .unwrap_err()
            .kind(),
            io::ErrorKind::TimedOut
        );
        assert_eq!(notifications, 1);
        assert_eq!(
            wait_for_owners(temp.path(), Duration::from_secs(2), || {
                Err(io::Error::other("fixture receipt failed"))
            })
            .unwrap_err()
            .to_string(),
            "fixture receipt failed"
        );
    }

    #[test]
    fn invalid_directories_and_owner_symlinks_fail_closed() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("agent-runtime");
        fs::write(&root, b"invalid directory").unwrap();
        assert!(wait_for_owners(temp.path(), Duration::ZERO, || Ok(())).is_err());
        fs::remove_file(&root).unwrap();
        symlink(temp.path(), &root).unwrap();
        assert!(wait_for_owners(temp.path(), Duration::ZERO, || Ok(())).is_err());
        fs::remove_file(&root).unwrap();
        fs::create_dir(&root).unwrap();
        symlink(temp.path().join("missing"), root.join("owner.lock")).unwrap();
        assert!(wait_for_owners(temp.path(), Duration::ZERO, || Ok(())).is_err());
        fs::remove_file(root.join("owner.lock")).unwrap();
        fs::create_dir(root.join("owner.lock")).unwrap();
        assert!(wait_for_owners(temp.path(), Duration::ZERO, || Ok(())).is_err());
    }

    #[test]
    fn early_relaunches_wait_for_readiness_and_coalesce() {
        let mut activation = Activation::default();
        for _ in 0..3 {
            activation.pending = true;
            assert!(!activation.schedule());
        }
        activation.ready = true;
        assert!(activation.schedule());
        assert!(!activation.schedule());
        assert!(activation.take());
        assert!(!activation.take());
        assert!(!activation.schedule());
        activation.pending = true;
        assert!(activation.schedule());
        assert!(activation.take());
    }

    #[test]
    fn shutdown_cancels_queued_activation_and_rejects_later_requests() {
        let mut activation = Activation {
            ready: true,
            pending: true,
            ..Default::default()
        };
        assert!(activation.schedule());
        activation.stop();
        assert!(!activation.take());
        activation.pending = true;
        assert!(!activation.schedule());
        assert!(!activation.take());
    }

    #[test]
    fn readiness_without_a_relaunch_does_not_present_again() {
        let mut activation = Activation {
            ready: true,
            ..Default::default()
        };
        assert!(!activation.schedule());
        assert!(!activation.take());
    }
}
