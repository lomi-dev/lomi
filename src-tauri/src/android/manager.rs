use super::{
    auth, avd,
    managed_adb::SharedServer,
    runtime::{DeviceRuntime, Phase, Status},
    storage::{Device, Directory},
};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tokio::sync::watch;

/// Tauri can construct this empty state at startup without filesystem work,
/// discovery, timers, JVM, ADB, or any Android child process.
#[derive(Default)]
pub struct Android {
    loaded: Mutex<Initialization>,
}

#[derive(Default)]
struct Initialization {
    manager: Option<Arc<Manager>>,
    exit: Option<String>,
    exit_ready: bool,
}

impl Android {
    pub fn get(&self, root: PathBuf) -> Result<Arc<Manager>, String> {
        let mut loaded = self
            .loaded
            .lock()
            .map_err(|_| "Android initialization failed")?;
        if let Some(manager) = &loaded.manager {
            return Ok(manager.clone());
        }
        if loaded.exit.is_some() {
            return Err(
                "Lomi is preparing to close. Cancel closing before setting up Android.".into(),
            );
        }
        let directory = Directory::acquire(root)?;
        let manager = Arc::new(Manager {
            directory: Arc::new(Mutex::new(directory)),
            core: Mutex::new(Core::default()),
            adb: Arc::new(Mutex::new(SharedServer::default())),
            installer: super::installer::Installer::default(),
            streams: super::frames::Streams::default(),
            input: super::input::Router::default(),
            #[cfg(unix)]
            agent_controls: Mutex::new(BTreeMap::new()),
            boot_gate: tokio::sync::Semaphore::new(1),
            emitter: std::sync::OnceLock::new(),
            #[cfg(any(test, feature = "android-probe", feature = "mcp-probe"))]
            test_adb_port: std::sync::atomic::AtomicU16::new({
                #[cfg(any(feature = "android-probe", feature = "mcp-probe"))]
                {
                    super::fixture::adb_port()
                }
                #[cfg(not(any(feature = "android-probe", feature = "mcp-probe")))]
                {
                    5037
                }
            }),
        });
        loaded.manager = Some(manager.clone());
        Ok(manager)
    }

    pub fn loaded(&self) -> Option<Arc<Manager>> {
        self.loaded.lock().ok()?.manager.clone()
    }

    pub fn begin_exit(&self) -> Result<String, String> {
        let mut state = self
            .loaded
            .lock()
            .map_err(|_| "Android initialization failed")?;
        if state.exit.is_some() {
            return Err("Application shutdown preparation is already active".into());
        }
        let token = match &state.manager {
            Some(manager) => manager.begin_exit()?,
            None => auth::new_id()?,
        };
        state.exit = Some(token.clone());
        state.exit_ready = false;
        Ok(token)
    }

    pub async fn finish_exit(&self, preparation: &str, force: bool) -> Result<(), String> {
        let manager = {
            let state = self
                .loaded
                .lock()
                .map_err(|_| "Android initialization failed")?;
            if state.exit.as_deref() != Some(preparation) {
                return Err("Stale application shutdown preparation".into());
            }
            state.manager.clone()
        };
        if let Some(manager) = manager {
            manager.stop_all(preparation, force).await?;
        }
        let mut state = self
            .loaded
            .lock()
            .map_err(|_| "Android initialization failed")?;
        if state.exit.as_deref() != Some(preparation) {
            return Err("Application shutdown was cancelled".into());
        }
        state.exit_ready = true;
        Ok(())
    }

    pub fn require_exit_ready(&self) -> Result<(), String> {
        let state = self
            .loaded
            .lock()
            .map_err(|_| "Android initialization failed")?;
        if state.exit_ready {
            Ok(())
        } else {
            Err("Prepare application shutdown before restarting Lomi.".into())
        }
    }

    /// Last-resort cleanup after the native event loop has committed to exit.
    /// Normal close, update and restart must settle through the frontend guards first.
    pub async fn emergency_cleanup(&self) -> Result<(), String> {
        let Some(manager) = self.loaded() else {
            return Ok(());
        };
        {
            let mut state = self
                .loaded
                .lock()
                .map_err(|_| "Android initialization failed")?;
            if state.exit.is_none() {
                state.exit = Some(manager.begin_exit()?);
            }
        }
        manager.emergency_stop_all().await?;
        #[cfg(any(feature = "android-probe", feature = "mcp-probe"))]
        if super::fixture::directory()?.is_some() {
            manager.stop_private_adb_fixture()?;
        }
        Ok(())
    }

    pub fn resume(&self, preparation: &str) -> Result<(), String> {
        let mut state = self
            .loaded
            .lock()
            .map_err(|_| "Android initialization failed")?;
        if state.exit.as_deref() != Some(preparation) {
            return Err("Stale application shutdown preparation".into());
        }
        if let Some(manager) = &state.manager {
            manager.resume(preparation)?;
        }
        state.exit = None;
        state.exit_ready = false;
        Ok(())
    }
}

pub struct Manager {
    #[cfg(unix)]
    agent_controls: Mutex<BTreeMap<String, Arc<lomi_control_core::android::AndroidControl>>>,
    pub directory: Arc<Mutex<Directory>>,
    pub installer: super::installer::Installer,
    pub streams: super::frames::Streams,
    pub input: super::input::Router,
    boot_gate: tokio::sync::Semaphore,
    core: Mutex<Core>,
    adb: Arc<Mutex<SharedServer>>,
    emitter: std::sync::OnceLock<Box<dyn Fn(super::events::Event) + Send + Sync>>,
    #[cfg(any(test, feature = "android-probe", feature = "mcp-probe"))]
    pub test_adb_port: std::sync::atomic::AtomicU16,
}

#[derive(Default)]
struct Core {
    devices: BTreeMap<String, (Device, DeviceRuntime)>,
    starting: BTreeMap<String, Arc<Starting>>,
    mutation: Option<String>,
    preparing_exit: Option<String>,
    settling_exit: bool,
}

struct Starting {
    guard: Option<super::runtime::DispatchGuard>,
    cancel: AtomicBool,
    result: watch::Sender<Option<Result<Status, String>>>,
}
impl Starting {
    async fn wait(&self) -> Result<Status, String> {
        let mut result = self.result.subscribe();
        loop {
            if let Some(value) = result.borrow_and_update().clone() {
                return value;
            }
            result
                .changed()
                .await
                .map_err(|_| "Android start ended without a result")?;
        }
    }
}

impl Core {
    fn editable(&self) -> Result<(), String> {
        if self.preparing_exit.is_some() {
            return Err(
                "Lomi is preparing to close. Finish or cancel that operation first.".into(),
            );
        }
        if self.mutation.is_some() {
            return Err("Android installation or device management is in progress. Wait or cancel it in Settings.".into());
        }
        Ok(())
    }
}

/// Held by the native operation task, never by the Settings window's lifetime.
pub struct Mutation {
    manager: Arc<Manager>,
    pub id: String,
}
impl Drop for Mutation {
    fn drop(&mut self) {
        if let Ok(mut core) = self.manager.core.lock() {
            if core.mutation.as_ref() == Some(&self.id) {
                core.mutation = None;
            }
        }
    }
}

impl Manager {
    #[cfg(unix)]
    pub fn agent_control(
        &self,
        device: &str,
    ) -> Option<Arc<lomi_control_core::android::AndroidControl>> {
        self.agent_controls.lock().ok()?.get(device).cloned()
    }

    #[cfg(unix)]
    pub fn revoke_agent(&self, device: &str) {
        if let Ok(controls) = self.agent_controls.lock() {
            if let Some(control) = controls.get(device) {
                control.revoke();
            }
        }
    }

    #[cfg(unix)]
    pub fn claim_agent(
        &self,
        control: Arc<lomi_control_core::android::AndroidControl>,
    ) -> Result<(), lomi_control_protocol::ErrorCode> {
        use lomi_control_protocol::ErrorCode;
        control.check()?;
        let mut controls = self
            .agent_controls
            .lock()
            .map_err(|_| ErrorCode::AppUnavailable)?;
        controls.retain(|_, current| current.check().is_ok());
        if let Some(current) = controls.get(&control.device) {
            return if Arc::ptr_eq(current, &control) {
                control.check()
            } else {
                Err(ErrorCode::TargetBusy)
            };
        }
        if controls.len() >= 16 {
            return Err(ErrorCode::ResourceExhausted);
        }
        let core = self.core.lock().map_err(|_| ErrorCode::AppUnavailable)?;
        core.editable().map_err(|_| ErrorCode::TargetBusy)?;
        if core.starting.contains_key(&control.device)
            || core
                .devices
                .get(&control.device)
                .is_some_and(|(_, runtime)| runtime.is_busy())
        {
            return Err(ErrorCode::TargetBusy);
        }
        control.check()?;
        controls.insert(control.device.clone(), control);
        Ok(())
    }

    #[cfg(unix)]
    pub fn agent_runtime(
        &self,
        control: &Arc<lomi_control_core::android::AndroidControl>,
        generation: &str,
    ) -> Result<DeviceRuntime, lomi_control_protocol::ErrorCode> {
        use lomi_control_protocol::ErrorCode;
        control.check_generation(generation)?;
        let controls = self
            .agent_controls
            .lock()
            .map_err(|_| ErrorCode::AppUnavailable)?;
        if !controls
            .get(&control.device)
            .is_some_and(|current| Arc::ptr_eq(current, control))
        {
            return Err(ErrorCode::ControlRevoked);
        }
        let core = self.core.lock().map_err(|_| ErrorCode::AppUnavailable)?;
        let runtime = &core
            .devices
            .get(&control.device)
            .ok_or(ErrorCode::TargetNotFound)?
            .1;
        if runtime.status().generation.as_deref() != Some(generation) {
            return Err(ErrorCode::StaleGeneration);
        }
        Ok(runtime.clone())
    }

    pub fn can_open(&self, id: &str) -> Result<(), String> {
        self.core
            .lock()
            .map_err(|_| "Android manager failed")?
            .editable()?;
        if !self
            .directory
            .lock()
            .map_err(|_| "Android directory failed")?
            .devices()?
            .devices
            .iter()
            .any(|device| device.id == id)
        {
            return Err("This Android device no longer exists. Refresh Settings.".into());
        }
        Ok(())
    }
    pub fn diagnostic_runtimes(&self) -> Result<Vec<(String, DeviceRuntime)>, String> {
        Ok(self
            .core
            .lock()
            .map_err(|_| "Android manager failed")?
            .devices
            .iter()
            .filter(|(_, (_, runtime))| runtime.status().process_alive)
            .take(32)
            .map(|(id, (_, runtime))| (id.clone(), runtime.clone()))
            .collect())
    }
    pub fn set_emitter(&self, emit: impl Fn(super::events::Event) + Send + Sync + 'static) {
        let _ = self.emitter.set(Box::new(emit));
    }

    pub fn emit(&self, event: super::events::Event) {
        if let Some(emit) = self.emitter.get() {
            emit(event);
        }
    }
    pub fn current_statuses(&self) -> Result<Vec<Status>, String> {
        let core = self.core.lock().map_err(|_| "Android manager failed")?;
        Ok(core
            .devices
            .values()
            .map(|(_, runtime)| runtime.status())
            .collect())
    }
    pub fn statuses(&self) -> Result<Vec<Status>, String> {
        let mut statuses = self.current_statuses()?;
        let known = statuses
            .iter()
            .map(|status| status.device_id.clone())
            .collect();
        let directory = self
            .directory
            .lock()
            .map_err(|_| "Android directory failed")?;
        statuses.extend(super::runtime::recovered_statuses(&directory.root, &known)?);
        Ok(statuses)
    }

    pub fn begin_mutation(self: &Arc<Self>, require_stopped: bool) -> Result<Mutation, String> {
        let mut core = self.core.lock().map_err(|_| "Android manager failed")?;
        core.editable()?;
        if !core.starting.is_empty()
            || (require_stopped && core.devices.values().any(|(_, runtime)| runtime.is_busy()))
        {
            return Err("Stop Android devices and wait for pending starts before changing these components.".into());
        }
        if require_stopped {
            let directory = self
                .directory
                .lock()
                .map_err(|_| "Android directory failed")?;
            super::runtime::require_no_recovered_process(&directory.root)?;
        }
        let id = auth::new_id()?;
        core.mutation = Some(id.clone());
        Ok(Mutation {
            manager: self.clone(),
            id,
        })
    }

    pub async fn start(self: &Arc<Self>, device: &str) -> Result<Status, String> {
        #[cfg(unix)]
        self.revoke_agent(device);
        self.start_guarded(device, None).await
    }

    pub async fn start_guarded(
        self: &Arc<Self>,
        device: &str,
        guard: Option<super::runtime::DispatchGuard>,
    ) -> Result<Status, String> {
        super::runtime::check_guard(&guard)?;
        if !super::storage::valid_id(device) {
            return Err("Invalid Android device ID".into());
        }
        let request = {
            let mut core = self.core.lock().map_err(|_| "Android manager failed")?;
            core.editable()?;
            if let Some(request) = core.starting.get(device) {
                super::runtime::check_guard(&request.guard)?;
                if request.guard.is_some() != guard.is_some() {
                    return Err(
                        "Android start is owned by another controller; wait for it to settle"
                            .into(),
                    );
                }
                request.clone()
            } else {
                let (result, _) = watch::channel(None);
                let request = Arc::new(Starting {
                    guard,
                    cancel: AtomicBool::new(false),
                    result,
                });
                core.starting.insert(device.into(), request.clone());
                let manager = self.clone();
                let id = device.to_owned();
                let task = request.clone();
                tokio::spawn(async move {
                    let result = manager.prepare_start(&id, &task).await;
                    if let Ok(mut core) = manager.core.lock() {
                        core.starting.remove(&id);
                    }
                    task.result.send_replace(Some(result));
                });
                request
            }
        };
        request.wait().await
    }

    async fn prepare_start(
        self: &Arc<Self>,
        id: &str,
        request: &Starting,
    ) -> Result<Status, String> {
        super::runtime::check_guard(&request.guard)?;
        let existing = {
            let core = self.core.lock().map_err(|_| "Android manager failed")?;
            if request.cancel.load(Ordering::Acquire) {
                return Err("Android start cancelled".into());
            }
            core.devices
                .get(id)
                .filter(|(_, runtime)| runtime.is_busy())
                .map(|(_, runtime)| runtime.request_start_guarded(request.guard.clone()))
                .transpose()?
        };
        if let Some(existing) = existing {
            return existing.await.map_err(|_| "Android start response ended")?;
        }
        let _boot = tokio::select! {
            permit = self.boot_gate.acquire() => permit.map_err(|_| "Android boot queue closed")?,
            _ = async {
                while !request.cancel.load(Ordering::Acquire) {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
            } => return Err("Android start cancelled while waiting for resources".into()),
        };
        super::runtime::check_guard(&request.guard)?;
        let (device, plan, adb) = {
            let directory = self
                .directory
                .lock()
                .map_err(|_| "Android directory failed")?;
            let device = directory.devices()?.devices.into_iter().find(|device| device.id == id).ok_or("This Android device is missing. Select another phone or repair it in Android settings.")?;
            super::runtime::check_previous(
                &directory.root.join("runtime").join(format!("{id}.json")),
                id,
            )?;
            super::devices::apply_stopped(&directory, &device)?;
            let plan = avd::launch(&directory, &device)?;
            #[cfg(any(test, feature = "android-probe", feature = "mcp-probe"))]
            let plan = super::runtime::Launch {
                adb_port: self.test_adb_port.load(Ordering::Acquire),
                ..plan
            };
            let adb = directory
                .installed_path("platform-tools")?
                .join(if cfg!(windows) { "adb.exe" } else { "adb" });
            (device, plan, adb)
        };
        if request.cancel.load(Ordering::Acquire) {
            return Err("Android start cancelled".into());
        }
        let root = plan.root.clone();
        let adb_port = plan.adb_port;
        let server = self.adb.clone();
        let guard = request.guard.clone();
        tokio::task::spawn_blocking(move || {
            super::runtime::check_guard(&guard)?;
            let mut server = server.lock().map_err(|_| "ADB startup failed")?;
            super::runtime::check_guard(&guard)?;
            if adb_port == 5037 {
                server.ensure(&root, &adb)
            } else {
                server.ensure_at(&root, &adb, adb_port)
            }
        })
        .await
        .map_err(|e| e.to_string())??;
        let result = {
            super::runtime::check_guard(&request.guard)?;
            let mut core = self.core.lock().map_err(|_| "Android manager failed")?;
            if request.cancel.load(Ordering::Acquire) {
                return Err("Android start cancelled".into());
            }
            let entry = core.devices.get(id);
            if entry.is_none_or(|(previous, runtime)| previous != &device && !runtime.is_busy()) {
                let runtime = DeviceRuntime::spawn(plan, self.directory.clone())?;
                let mut changes = runtime.subscribe();
                let manager = Arc::downgrade(self);
                tokio::spawn(async move {
                    loop {
                        let Some(manager) = manager.upgrade() else {
                            break;
                        };
                        manager.emit(super::events::Event::Status(
                            changes.borrow_and_update().clone(),
                        ));
                        drop(manager);
                        if changes.changed().await.is_err() {
                            break;
                        }
                    }
                });
                core.devices.insert(id.into(), (device, runtime));
            }
            core.devices
                .get(id)
                .ok_or("Android runtime was not created")?
                .1
                .request_start_guarded(request.guard.clone())?
        };
        result.await.map_err(|_| "Android start response ended")?
    }

    #[cfg(any(test, feature = "android-probe", feature = "mcp-probe"))]
    pub fn stop_private_adb_fixture(&self) -> Result<(), String> {
        self.adb
            .lock()
            .unwrap()
            .stop_private_fixture(self.test_adb_port.load(Ordering::Acquire))
    }

    pub async fn stop(&self, id: &str, force: bool) -> Result<Status, String> {
        #[cfg(unix)]
        self.revoke_agent(id);
        if !super::storage::valid_id(id) {
            return Err("Invalid Android device ID".into());
        }
        let (starting, runtime) = {
            let core = self.core.lock().map_err(|_| "Android manager failed")?;
            let starting = core.starting.get(id).cloned();
            if let Some(starting) = &starting {
                starting.cancel.store(true, Ordering::Release);
            }
            (
                starting,
                core.devices.get(id).map(|(_, runtime)| runtime.clone()),
            )
        };
        // A pending preparation either forwarded Start before the locked snapshot,
        // or observes cancel and cannot create a process after the last view closes.
        let stopped = match runtime {
            Some(runtime) => runtime.stop(force).await,
            None => {
                let root = self
                    .directory
                    .lock()
                    .map_err(|_| "Android directory failed")?
                    .root
                    .clone();
                super::runtime::stop_recovered(root, id.into(), force).await
            }
        };
        if let Some(starting) = starting {
            let _ = starting.wait().await;
        }
        if let Ok(status) = &stopped {
            self.emit(super::events::Event::Status(status.clone()));
        }
        stopped
    }

    pub fn runtime(&self, id: &str, generation: &str) -> Result<DeviceRuntime, String> {
        let core = self.core.lock().map_err(|_| "Android manager failed")?;
        let runtime = &core.devices.get(id).ok_or("Android is not running")?.1;
        let status = runtime.status();
        if status.phase != Phase::Running || status.generation.as_deref() != Some(generation) {
            return Err(
                "This Android action belongs to an old instance. Reconnect the panel.".into(),
            );
        }
        Ok(runtime.clone())
    }

    pub fn begin_exit(&self) -> Result<String, String> {
        let mut core = self.core.lock().map_err(|_| "Android manager failed")?;
        if core.preparing_exit.is_some() {
            return Err("Android shutdown preparation is already in progress".into());
        }
        let id = auth::new_id()?;
        core.preparing_exit = Some(id.clone());
        Ok(id)
    }

    pub fn resume(&self, id: &str) -> Result<(), String> {
        let mut core = self.core.lock().map_err(|_| "Android manager failed")?;
        if core.preparing_exit.as_deref() != Some(id) {
            return Err("Stale Android shutdown preparation".into());
        }
        if core.settling_exit {
            return Err(
                "Android shutdown is still settling. Wait before cancelling preparation.".into(),
            );
        }
        core.preparing_exit = None;
        Ok(())
    }

    pub async fn stop_all(self: &Arc<Self>, preparation: &str, force: bool) -> Result<(), String> {
        let recovered = self.statuses()?.into_iter().map(|status| status.device_id);
        let devices = {
            let mut core = self.core.lock().map_err(|_| "Android manager failed")?;
            if core.preparing_exit.as_deref() != Some(preparation) || core.settling_exit {
                return Err("Invalid or already active Android shutdown preparation".into());
            }
            core.settling_exit = true;
            core.devices
                .keys()
                .chain(core.starting.keys())
                .cloned()
                .chain(recovered)
                .collect::<std::collections::BTreeSet<_>>()
        };
        // The owned task settles even if an invoke or updater future is dropped.
        let (send, receive) = tokio::sync::oneshot::channel();
        let manager = self.clone();
        tokio::spawn(async move {
            let result = async {
                manager.installer.settle(true).await?;
                if manager
                    .core
                    .lock()
                    .map_err(|_| "Android manager failed")?
                    .mutation
                    .is_some()
                {
                    return Err(
                        "Android metadata is still being saved. Retry closing when it finishes."
                            .into(),
                    );
                }
                for id in devices {
                    manager.stop(&id, force).await?;
                }
                Ok(())
            }
            .await;
            if let Ok(mut core) = manager.core.lock() {
                core.settling_exit = false;
            }
            let _ = send.send(result);
        });
        receive
            .await
            .map_err(|_| "Android shutdown result unavailable")?
    }

    async fn emergency_stop_all(&self) -> Result<(), String> {
        let mut errors = Vec::new();
        if let Err(error) = self.installer.settle(true).await {
            errors.push(error);
        }
        let mut devices = {
            let core = self.core.lock().map_err(|_| "Android manager failed")?;
            core.devices
                .keys()
                .chain(core.starting.keys())
                .cloned()
                .collect::<std::collections::BTreeSet<_>>()
        };
        match self.statuses() {
            Ok(statuses) => devices.extend(statuses.into_iter().map(|s| s.device_id)),
            Err(error) => errors.push(error),
        }
        // An unreadable recovery record or one failed child must not prevent
        // attempts to reap other retained children after native exit is committed.
        for id in devices {
            if self.stop(&id, false).await.is_err() {
                if let Err(error) = self.stop(&id, true).await {
                    errors.push(format!("{id}: {error}"));
                }
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn revoked_agent_boot_leaves_the_preparation_queue_without_starting_tools() {
        let root = tempfile::tempdir().unwrap();
        let manager = Android::default().get(root.path().join("android")).unwrap();
        let occupied = manager.boot_gate.acquire().await.unwrap();
        let allowed = Arc::new(AtomicBool::new(true));
        let active = allowed.clone();
        let guard: super::super::runtime::DispatchGuard = Arc::new(move || {
            if active.load(Ordering::SeqCst) {
                Ok(())
            } else {
                Err("revoked fixture authority".into())
            }
        });
        let starting = manager.clone();
        let start = tokio::spawn(async move {
            starting
                .start_guarded("00000000-0000-0000-0000-000000000001", Some(guard))
                .await
        });
        while manager.core.lock().unwrap().starting.is_empty() {
            tokio::task::yield_now().await;
        }
        allowed.store(false, Ordering::SeqCst);
        drop(occupied);
        let result = tokio::time::timeout(std::time::Duration::from_secs(2), start)
            .await
            .unwrap()
            .unwrap();
        assert!(result.unwrap_err().contains("revoked fixture"));
        assert!(manager.core.lock().unwrap().starting.is_empty());
        assert!(!root.path().join("android/sdk").exists());
    }

    #[tokio::test]
    async fn queued_boot_can_be_cancelled_before_preparation_without_spawning_tools() {
        let root = tempfile::tempdir().unwrap();
        let android = Android::default();
        let manager = android.get(root.path().join("android")).unwrap();
        let _occupied = manager.boot_gate.acquire().await.unwrap();
        let id = "00000000-0000-0000-0000-000000000001";
        let starting = manager.clone();
        let start = tokio::spawn(async move { starting.start(id).await });
        while manager.core.lock().unwrap().starting.is_empty() {
            tokio::task::yield_now().await;
        }
        let result =
            tokio::time::timeout(std::time::Duration::from_secs(2), manager.stop(id, false))
                .await
                .unwrap()
                .unwrap();
        assert!(!result.process_alive);
        assert!(start.await.unwrap().unwrap_err().contains("cancelled"));
        assert!(!root.path().join("android/sdk").exists());
    }

    #[tokio::test]
    async fn exit_before_first_android_use_blocks_late_initialization_without_touching_disk() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("android");
        let state = Android::default();
        assert!(state.require_exit_ready().is_err());
        let preparation = state.begin_exit().unwrap();
        assert!(state.require_exit_ready().is_err());
        assert!(state.get(root.clone()).is_err());
        state.finish_exit(&preparation, false).await.unwrap();
        state.require_exit_ready().unwrap();
        assert!(!root.exists());
        assert!(state.loaded().is_none());
        state.resume(&preparation).unwrap();
        assert!(state.require_exit_ready().is_err());
        assert!(state.get(root).is_ok());
    }

    #[test]
    fn directory_owner_is_lazy_and_exit_gate_does_not_release_it() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("android");
        let android = Android::default();
        assert!(android.loaded().is_none());
        assert!(!path.exists());
        let manager = android.get(path.clone()).unwrap();
        assert!(Directory::acquire(path.clone()).is_err());
        let operation = manager.begin_mutation(false).unwrap();
        assert!(manager.begin_mutation(false).is_err());
        let close = manager.begin_exit().unwrap();
        assert!(manager.begin_mutation(false).is_err());
        drop(operation);
        manager.resume(&close).unwrap();
        assert!(manager.resume(&close).is_err());
        assert!(manager.begin_mutation(true).is_ok());
        assert!(manager.statuses().unwrap().is_empty());
        assert!(Directory::acquire(path).is_err());
    }

    #[tokio::test]
    async fn missing_device_and_cancelled_exit_never_start_tools() {
        let root = tempfile::tempdir().unwrap();
        let android = Android::default();
        let manager = android.get(root.path().join("android")).unwrap();
        let id = "00000000-0000-0000-0000-000000000001";
        assert!(manager.start(id).await.unwrap_err().contains("missing"));
        assert!(manager.runtime(id, id).is_err());
        assert!(!manager.stop(id, false).await.unwrap().process_alive);
        let close = manager.begin_exit().unwrap();
        assert!(manager.start(id).await.unwrap_err().contains("preparing"));
        manager.stop_all(&close, false).await.unwrap();
        manager.resume(&close).unwrap();
        assert!(manager.start(id).await.unwrap_err().contains("missing"));
        assert!(!root.path().join("android/sdk").exists());
    }
}
